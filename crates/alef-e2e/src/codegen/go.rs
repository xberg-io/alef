//! Go e2e test generator using testing.T.

use crate::config::E2eConfig;
use crate::escape::{go_string_literal, sanitize_filename};
use crate::field_access::FieldResolver;
use crate::fixture::{Assertion, CallbackAction, Fixture, FixtureGroup, ValidationErrorExpectation};
use alef_codegen::naming::{go_param_name, to_go_name};
use alef_core::backend::GeneratedFile;
use alef_core::config::Language;
use alef_core::config::ResolvedCrateConfig;
use alef_core::hash::{self, CommentStyle};
use anyhow::Result;
use heck::ToUpperCamelCase;
use std::fmt::Write as FmtWrite;
use std::path::PathBuf;

use super::E2eCodegen;
use super::client;

/// Go e2e code generator.
pub struct GoCodegen;

impl E2eCodegen for GoCodegen {
    fn generate(
        &self,
        groups: &[FixtureGroup],
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
    ) -> Result<Vec<GeneratedFile>> {
        let lang = self.language_name();
        let output_base = PathBuf::from(e2e_config.effective_output()).join(lang);

        let mut files = Vec::new();

        // Resolve call config with overrides (for module path and import alias).
        let call = &e2e_config.call;
        let overrides = call.overrides.get(lang);
        let configured_go_module_path = config.go.as_ref().and_then(|go| go.module.as_ref()).cloned();
        let module_path = overrides
            .and_then(|o| o.module.as_ref())
            .cloned()
            .or_else(|| configured_go_module_path.clone())
            .unwrap_or_else(|| call.module.clone());
        let import_alias = overrides
            .and_then(|o| o.alias.as_ref())
            .cloned()
            .unwrap_or_else(|| "pkg".to_string());

        // Resolve package config.
        let go_pkg = e2e_config.resolve_package("go");
        let go_module_path = go_pkg
            .as_ref()
            .and_then(|p| p.module.as_ref())
            .cloned()
            .or_else(|| configured_go_module_path.clone())
            .unwrap_or_else(|| module_path.clone());
        let replace_path = go_pkg
            .as_ref()
            .and_then(|p| p.path.as_ref())
            .cloned()
            .or_else(|| Some(format!("../../{}", config.package_dir(Language::Go))));
        let go_version = go_pkg
            .as_ref()
            .and_then(|p| p.version.as_ref())
            .cloned()
            .unwrap_or_else(|| {
                config
                    .resolved_version()
                    .map(|v| format!("v{v}"))
                    .unwrap_or_else(|| "v0.0.0".to_string())
            });
        let field_resolver = FieldResolver::new(
            &e2e_config.fields,
            &e2e_config.fields_optional,
            &e2e_config.result_fields,
            &e2e_config.fields_array,
        );

        // Generate go.mod. In registry mode, omit the `replace` directive so the
        // module is fetched from the Go module proxy.
        let effective_replace = match e2e_config.dep_mode {
            crate::config::DependencyMode::Registry => None,
            crate::config::DependencyMode::Local => replace_path.as_deref().map(String::from),
        };
        // In local mode with a `replace` directive the version in `require` is a
        // placeholder.  Go requires that for a major-version module path (`/vN`, N ≥ 2)
        // the placeholder version must start with `vN.`, e.g. `v3.0.0`.  A version like
        // `v0.0.0` is rejected with "should be v3, not v0".  Fix the placeholder when the
        // module path ends with `/vN` and the configured version doesn't match.
        let effective_go_version = if effective_replace.is_some() {
            fix_go_major_version(&go_module_path, &go_version)
        } else {
            go_version.clone()
        };
        files.push(GeneratedFile {
            path: output_base.join("go.mod"),
            content: render_go_mod(&go_module_path, effective_replace.as_deref(), &effective_go_version),
            generated_header: false,
        });

        // Generate test files per category.
        for group in groups {
            let active: Vec<&Fixture> = group
                .fixtures
                .iter()
                .filter(|f| f.skip.as_ref().is_none_or(|s| !s.should_skip(lang)))
                .collect();

            if active.is_empty() {
                continue;
            }

            let filename = format!("{}_test.go", sanitize_filename(&group.category));
            let content = render_test_file(
                &group.category,
                &active,
                &module_path,
                &import_alias,
                &field_resolver,
                e2e_config,
            );
            files.push(GeneratedFile {
                path: output_base.join(filename),
                content,
                generated_header: true,
            });
        }

        Ok(files)
    }

    fn language_name(&self) -> &'static str {
        "go"
    }
}

/// Fix a Go module version so it is valid for a major-version module path.
///
/// Go requires that a module path ending in `/vN` (N ≥ 2) uses a version
/// whose major component matches N.  In local-replace mode we use a synthetic
/// placeholder version; if that placeholder (e.g. `v0.0.0`) doesn't match the
/// major suffix, fix it to `vN.0.0` so `go mod` accepts the go.mod.
fn fix_go_major_version(module_path: &str, version: &str) -> String {
    // Extract `/vN` suffix from the module path (N must be ≥ 2).
    let major = module_path
        .rsplit('/')
        .next()
        .and_then(|seg| seg.strip_prefix('v'))
        .and_then(|n| n.parse::<u64>().ok())
        .filter(|&n| n >= 2);

    let Some(n) = major else {
        return version.to_string();
    };

    // If the version already starts with `vN.`, it is valid — leave it alone.
    let expected_prefix = format!("v{n}.");
    if version.starts_with(&expected_prefix) {
        return version.to_string();
    }

    format!("v{n}.0.0")
}

fn render_go_mod(go_module_path: &str, replace_path: Option<&str>, version: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "module e2e_go");
    let _ = writeln!(out);
    let _ = writeln!(out, "go 1.26");
    let _ = writeln!(out);
    let _ = writeln!(out, "require (");
    let _ = writeln!(out, "\t{go_module_path} {version}");
    let _ = writeln!(out, "\tgithub.com/stretchr/testify v1.11.1");
    let _ = writeln!(out, ")");

    if let Some(path) = replace_path {
        let _ = writeln!(out);
        let _ = writeln!(out, "replace {go_module_path} => {path}");
    }

    out
}

fn render_test_file(
    category: &str,
    fixtures: &[&Fixture],
    go_module_path: &str,
    import_alias: &str,
    field_resolver: &FieldResolver,
    e2e_config: &crate::config::E2eConfig,
) -> String {
    let mut out = String::new();
    let emits_executable_test =
        |fixture: &Fixture| fixture.is_http_test() || fixture_has_go_callable(fixture, e2e_config);

    // Go convention: generated file marker must appear before the package declaration.
    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    let _ = writeln!(out);

    // Determine if any fixture actually uses the pkg import.
    // Fixtures without mock_response are emitted as t.Skip() stubs and don't reference the
    // package — omit the import when no fixture needs it to avoid the Go "imported and not
    // used" compile error. Visitor fixtures reference the package types (NodeContext,
    // VisitResult, VisitResult* helpers) in struct method signatures emitted at file scope,
    // so they also require the import even when the test body itself is a Skip stub.
    // Direct-callable fixtures (non-HTTP, non-mock, with a resolved Go function) also
    // reference the package when a Go override function is configured.
    let needs_pkg = fixtures
        .iter()
        .any(|f| f.mock_response.is_some() || f.visitor.is_some() || fixture_has_go_callable(f, e2e_config));

    // Determine if we need the "os" import (mock_url args, HTTP fixtures, or
    // client_factory fixtures that read MOCK_SERVER_URL via os.Getenv).
    let needs_os = fixtures.iter().any(|f| {
        if f.is_http_test() {
            return true;
        }
        if !emits_executable_test(f) {
            return false;
        }
        let call_config = e2e_config.resolve_call(f.call.as_deref());
        let go_override = call_config
            .overrides
            .get("go")
            .or_else(|| e2e_config.call.overrides.get("go"));
        if go_override.and_then(|o| o.client_factory.as_deref()).is_some() {
            return true;
        }
        let call_args = &call_config.args;
        call_args.iter().any(|a| a.arg_type == "mock_url")
    });

    let needs_json_stringify = fixtures.iter().any(|f| {
        emits_executable_test(f)
            && f.assertions.iter().any(|a| {
                matches!(
                    a.assertion_type.as_str(),
                    "contains" | "contains_all" | "contains_any" | "not_contains"
                ) && a
                    .field
                    .as_ref()
                    .map(|f| field_resolver.is_array(field_resolver.resolve(f)))
                    .unwrap_or(false)
            })
    });

    // Determine if we need "encoding/json" (handle args with non-null config,
    // json_object args that will be unmarshalled into a typed struct, or HTTP
    // body/partial/validation-error assertions that use json.Unmarshal).
    let needs_json = needs_json_stringify
        || fixtures.iter().any(|f| {
            // HTTP body assertions use json.Unmarshal for Object/Array bodies;
            // partial body and validation-error assertions always use json.Unmarshal.
            if let Some(http) = &f.http {
                let body_needs_json = http
                    .expected_response
                    .body
                    .as_ref()
                    .is_some_and(|b| matches!(b, serde_json::Value::Object(_) | serde_json::Value::Array(_)));
                let partial_needs_json = http.expected_response.body_partial.is_some();
                let ve_needs_json = http
                    .expected_response
                    .validation_errors
                    .as_ref()
                    .is_some_and(|v| !v.is_empty());
                if body_needs_json || partial_needs_json || ve_needs_json {
                    return true;
                }
            }
            if !emits_executable_test(f) {
                return false;
            }

            let call = e2e_config.resolve_call(f.call.as_deref());
            let call_args = &call.args;
            // handle args with non-null config value
            let has_handle = call_args.iter().any(|a| a.arg_type == "handle") && {
                call_args.iter().filter(|a| a.arg_type == "handle").any(|a| {
                    let field = a.field.strip_prefix("input.").unwrap_or(&a.field);
                    let v = f.input.get(field).unwrap_or(&serde_json::Value::Null);
                    !(v.is_null() || v.is_object() && v.as_object().is_some_and(|o| o.is_empty()))
                })
            };
            // json_object args with options_type or array values (will use JSON unmarshal)
            let go_override = call.overrides.get("go");
            let opts_type = go_override.and_then(|o| o.options_type.as_deref()).or_else(|| {
                e2e_config
                    .call
                    .overrides
                    .get("go")
                    .and_then(|o| o.options_type.as_deref())
            });
            let has_json_obj = call_args.iter().any(|a| {
                if a.arg_type != "json_object" {
                    return false;
                }
                let field = a.field.strip_prefix("input.").unwrap_or(&a.field);
                let v = f.input.get(field).unwrap_or(&serde_json::Value::Null);
                if v.is_array() {
                    return true;
                } // array → []string unmarshal
                opts_type.is_some() && v.is_object() && !v.as_object().is_some_and(|o| o.is_empty())
            });
            has_handle || has_json_obj
        });

    // Determine if we need "encoding/base64" (bytes-type args decoded at runtime).
    let needs_base64 = fixtures.iter().any(|f| {
        if !emits_executable_test(f) {
            return false;
        }
        let call_args = &e2e_config.resolve_call(f.call.as_deref()).args;
        call_args.iter().any(|a| {
            if a.arg_type != "bytes" {
                return false;
            }
            let field = a.field.strip_prefix("input.").unwrap_or(&a.field);
            matches!(f.input.get(field), Some(serde_json::Value::String(_)))
        })
    });

    // Determine if we need the "fmt" import (CustomTemplate visitor actions
    // with placeholders, or string assertions rendered through fmt.Sprint so
    // structured slices can be searched without assuming []string).
    let needs_fmt = fixtures.iter().any(|f| {
        f.visitor.as_ref().is_some_and(|v| {
            v.callbacks.values().any(|action| {
                if let CallbackAction::CustomTemplate { template } = action {
                    template.contains('{')
                } else {
                    false
                }
            })
        }) || (emits_executable_test(f)
            && f.assertions.iter().any(|a| {
                matches!(
                    a.assertion_type.as_str(),
                    "contains" | "contains_all" | "contains_any" | "not_contains"
                ) && a
                    .field
                    .as_ref()
                    .map(|f| f.is_empty() || field_resolver.is_valid_for_result(f))
                    .unwrap_or(true)
            }))
    });

    // Determine if we need the "strings" import.
    // Only count assertions whose fields are actually valid for the result type.
    let needs_strings = fixtures.iter().any(|f| {
        if !emits_executable_test(f) {
            return false;
        }
        f.assertions.iter().any(|a| {
            let type_needs_strings = if a.assertion_type == "equals" {
                // equals with string values needs strings.TrimSpace
                a.value.as_ref().is_some_and(|v| v.is_string())
            } else {
                matches!(
                    a.assertion_type.as_str(),
                    "contains" | "contains_all" | "contains_any" | "not_contains" | "starts_with" | "ends_with"
                )
            };
            let field_valid = a
                .field
                .as_ref()
                .map(|f| f.is_empty() || field_resolver.is_valid_for_result(f))
                .unwrap_or(true);
            type_needs_strings && field_valid
        })
    });

    // Determine if we need the testify assert import.
    let needs_assert = fixtures.iter().any(|f| {
        if !emits_executable_test(f) {
            return false;
        }
        f.assertions.iter().any(|a| {
            let field_valid = a
                .field
                .as_ref()
                .map(|f| f.is_empty() || field_resolver.is_valid_for_result(f))
                .unwrap_or(true);
            let synthetic_field_needs_assert = match a.field.as_deref() {
                Some("chunks_have_content" | "chunks_have_embeddings") => {
                    matches!(a.assertion_type.as_str(), "is_true" | "is_false")
                }
                Some("embeddings") => {
                    matches!(
                        a.assertion_type.as_str(),
                        "count_equals" | "count_min" | "not_empty" | "is_empty"
                    )
                }
                _ => false,
            };
            let type_needs_assert = matches!(
                a.assertion_type.as_str(),
                "count_equals"
                    | "count_min"
                    | "count_max"
                    | "is_true"
                    | "is_false"
                    | "method_result"
                    | "min_length"
                    | "max_length"
                    | "matches_regex"
            );
            synthetic_field_needs_assert || type_needs_assert && field_valid
        })
    });

    // Determine if we need "net/http" and "io" (HTTP server tests via HTTP client).
    let has_http_fixtures = fixtures.iter().any(|f| f.is_http_test());
    let needs_http = has_http_fixtures;
    // io.ReadAll is emitted for every HTTP fixture (render_call always reads the body).
    let needs_io = has_http_fixtures;

    // Determine if we need "reflect" (for HTTP response body JSON comparison
    // and partial-body assertions, both of which use reflect.DeepEqual).
    let needs_reflect = fixtures.iter().any(|f| {
        if let Some(http) = &f.http {
            let body_needs_reflect = http
                .expected_response
                .body
                .as_ref()
                .is_some_and(|b| matches!(b, serde_json::Value::Object(_) | serde_json::Value::Array(_)));
            let partial_needs_reflect = http.expected_response.body_partial.is_some();
            body_needs_reflect || partial_needs_reflect
        } else {
            false
        }
    });

    let _ = writeln!(out, "// E2e tests for category: {category}");
    let _ = writeln!(out, "package e2e_test");
    let _ = writeln!(out);
    let _ = writeln!(out, "import (");
    if needs_base64 {
        let _ = writeln!(out, "\t\"encoding/base64\"");
    }
    if needs_json || needs_reflect {
        let _ = writeln!(out, "\t\"encoding/json\"");
    }
    if needs_fmt {
        let _ = writeln!(out, "\t\"fmt\"");
    }
    if needs_io {
        let _ = writeln!(out, "\t\"io\"");
    }
    if needs_http {
        let _ = writeln!(out, "\t\"net/http\"");
    }
    if needs_os {
        let _ = writeln!(out, "\t\"os\"");
    }
    if needs_reflect {
        let _ = writeln!(out, "\t\"reflect\"");
    }
    if needs_strings {
        let _ = writeln!(out, "\t\"strings\"");
    }
    let _ = writeln!(out, "\t\"testing\"");
    if needs_assert {
        let _ = writeln!(out);
        let _ = writeln!(out, "\t\"github.com/stretchr/testify/assert\"");
    }
    if needs_pkg {
        let _ = writeln!(out);
        let _ = writeln!(out, "\t{import_alias} \"{go_module_path}\"");
    }
    let _ = writeln!(out, ")");
    let _ = writeln!(out);

    if needs_json_stringify {
        let _ = writeln!(out, "func jsonString(value any) string {{");
        let _ = writeln!(out, "\tencoded, err := json.Marshal(value)");
        let _ = writeln!(out, "\tif err != nil {{");
        let _ = writeln!(out, "\t\treturn fmt.Sprint(value)");
        let _ = writeln!(out, "\t}}");
        let _ = writeln!(out, "\treturn string(encoded)");
        let _ = writeln!(out, "}}");
        let _ = writeln!(out);
    }

    // Emit package-level visitor structs (must be outside any function in Go).
    for fixture in fixtures.iter() {
        if let Some(visitor_spec) = &fixture.visitor {
            let struct_name = visitor_struct_name(&fixture.id);
            emit_go_visitor_struct(&mut out, &struct_name, visitor_spec, import_alias);
            let _ = writeln!(out);
        }
    }

    for (i, fixture) in fixtures.iter().enumerate() {
        render_test_function(&mut out, fixture, import_alias, field_resolver, e2e_config);
        if i + 1 < fixtures.len() {
            let _ = writeln!(out);
        }
    }

    // Clean up trailing newlines.
    while out.ends_with("\n\n") {
        out.pop();
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Return `true` when a non-HTTP fixture can be exercised by calling the Go
/// binding directly.
///
/// A fixture is Go-callable when the resolved call config provides a non-empty
/// function name — either via a Go-specific override (`[e2e.call.overrides.go]
/// function`) or via the base call `function` field.  The Go binding exposes all
/// public functions from the Rust core as PascalCase exports, so any non-empty
/// function name can be resolved to a valid Go symbol via `to_go_name`.
fn fixture_has_go_callable(fixture: &Fixture, e2e_config: &crate::config::E2eConfig) -> bool {
    // HTTP fixtures are handled by render_http_test_function — not our concern here.
    if fixture.is_http_test() {
        return false;
    }
    let call_config = e2e_config.resolve_call(fixture.call.as_deref());
    let go_override = call_config
        .overrides
        .get("go")
        .or_else(|| e2e_config.call.overrides.get("go"));
    // When a client_factory is configured, the fixture is callable via the
    // client-method pattern even when the base function name is empty.
    if go_override.and_then(|o| o.client_factory.as_deref()).is_some() {
        return true;
    }
    // Prefer a Go-specific override function name; fall back to the base function name.
    // Any non-empty function name is callable: the Go binding exports all public
    // Rust functions as PascalCase symbols (snake_case → PascalCase via to_go_name).
    let fn_name = go_override
        .and_then(|o| o.function.as_deref())
        .filter(|s| !s.is_empty())
        .unwrap_or(call_config.function.as_str());
    !fn_name.is_empty()
}

fn render_test_function(
    out: &mut String,
    fixture: &Fixture,
    import_alias: &str,
    field_resolver: &FieldResolver,
    e2e_config: &crate::config::E2eConfig,
) {
    let fn_name = fixture.id.to_upper_camel_case();
    let description = &fixture.description;

    // Delegate HTTP fixtures to the shared driver via GoTestClientRenderer.
    if fixture.http.is_some() {
        render_http_test_function(out, fixture);
        return;
    }

    // Non-HTTP, non-mock fixtures can be tested directly if the call config
    // provides a callable Go function (via [e2e.call.overrides.go] `function`
    // or the base call `function`). Only emit a t.Skip stub when there is no
    // usable callable — this keeps the package compilable while being honest
    // about what can and cannot be exercised.
    if fixture.mock_response.is_none() && !fixture_has_go_callable(fixture, e2e_config) {
        let _ = writeln!(out, "func Test_{fn_name}(t *testing.T) {{");
        let _ = writeln!(out, "\t// {description}");
        let _ = writeln!(
            out,
            "\tt.Skip(\"non-HTTP fixture: Go binding does not expose a callable for the configured `[e2e.call]` function\")"
        );
        let _ = writeln!(out, "}}");
        return;
    }

    // Resolve call config per-fixture (supports named calls via fixture.call).
    let call_config = e2e_config.resolve_call(fixture.call.as_deref());
    let lang = "go";
    let overrides = call_config.overrides.get(lang);

    // Select the function name: when the fixture includes a visitor and a
    // `visitor_function` override is configured, use the visitor-accepting
    // entry point (e.g. `ConvertWithVisitor`) instead of the plain function.
    let base_function_name = if fixture.visitor.is_some() {
        overrides
            .and_then(|o| o.visitor_function.as_deref())
            .or_else(|| {
                e2e_config
                    .call
                    .overrides
                    .get(lang)
                    .and_then(|o| o.visitor_function.as_deref())
            })
            .unwrap_or_else(|| {
                overrides
                    .and_then(|o| o.function.as_deref())
                    .unwrap_or(&call_config.function)
            })
    } else {
        overrides
            .and_then(|o| o.function.as_deref())
            .unwrap_or(&call_config.function)
    };
    let function_name = to_go_name(base_function_name);
    let result_var = &call_config.result_var;
    let args = &call_config.args;

    // Whether the function returns (value, error) or just (error) or just (value).
    // Check Go override first, fall back to call-level returns_result.
    let returns_result = overrides
        .and_then(|o| o.returns_result)
        .unwrap_or(call_config.returns_result);

    // Whether the function returns only error (no value component), i.e. Result<(), E>.
    // When returns_result=true and returns_void=true, Go emits `err :=` not `_, err :=`.
    let returns_void = call_config.returns_void;

    // result_is_simple: result is a scalar (*string, *bool, etc.) not a struct.
    // Priority: Go override > call-level (canonical source) > Rust override (legacy compat).
    let result_is_simple = overrides.map(|o| o.result_is_simple).unwrap_or_else(|| {
        if call_config.result_is_simple {
            return true;
        }
        call_config
            .overrides
            .get("rust")
            .map(|o| o.result_is_simple)
            .unwrap_or(false)
    });

    // result_is_array: the simple result is a slice/array type (e.g., []string).
    // Only relevant when result_is_simple is true.
    let result_is_array = overrides.map(|o| o.result_is_array).unwrap_or(false);

    // Per-call Go options_type, falling back to the default call's Go override.
    let call_options_type = overrides.and_then(|o| o.options_type.as_deref()).or_else(|| {
        e2e_config
            .call
            .overrides
            .get("go")
            .and_then(|o| o.options_type.as_deref())
    });

    // Whether json_object options are passed as a pointer (*OptionsType).
    let call_options_ptr = overrides.map(|o| o.options_ptr).unwrap_or_else(|| {
        e2e_config
            .call
            .overrides
            .get("go")
            .map(|o| o.options_ptr)
            .unwrap_or(false)
    });

    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");

    // Client factory: when set, the test creates a client via `pkg.Factory("test-key", baseURL)`
    // and calls methods on the instance rather than top-level package functions.
    let client_factory = overrides.and_then(|o| o.client_factory.as_deref()).or_else(|| {
        e2e_config
            .call
            .overrides
            .get(lang)
            .and_then(|o| o.client_factory.as_deref())
    });

    let (mut setup_lines, args_str) = build_args_and_setup(
        &fixture.input,
        args,
        import_alias,
        call_options_type,
        &fixture.id,
        call_options_ptr,
    );

    // Build visitor if present — struct is at package level, just instantiate here.
    let mut visitor_arg = String::new();
    if fixture.visitor.is_some() {
        let struct_name = visitor_struct_name(&fixture.id);
        setup_lines.push(format!("visitor := &{struct_name}{{}}"));
        visitor_arg = "visitor".to_string();
    }

    let final_args = if visitor_arg.is_empty() {
        args_str
    } else {
        format!("{args_str}, {visitor_arg}")
    };

    let _ = writeln!(out, "func Test_{fn_name}(t *testing.T) {{");
    let _ = writeln!(out, "\t// {description}");

    for line in &setup_lines {
        let _ = writeln!(out, "\t{line}");
    }

    // Client factory: emit client creation before the call.
    // Each test creates a fresh client pointed at MOCK_SERVER_URL/fixtures/<id>
    // so the mock server can serve the fixture response via prefix routing.
    let call_prefix = if let Some(factory) = client_factory {
        let factory_name = to_go_name(factory);
        let fixture_id = &fixture.id;
        let _ = writeln!(
            out,
            "\tmockURL := os.Getenv(\"MOCK_SERVER_URL\") + \"/fixtures/{fixture_id}\""
        );
        let _ = writeln!(
            out,
            "\tclient, clientErr := {import_alias}.{factory_name}(\"test-key\", &mockURL, nil, nil, nil)"
        );
        let _ = writeln!(out, "\tif clientErr != nil {{");
        let _ = writeln!(out, "\t\tt.Fatalf(\"create client failed: %v\", clientErr)");
        let _ = writeln!(out, "\t}}");
        "client".to_string()
    } else {
        import_alias.to_string()
    };

    // The Go binding generator wraps the FFI call in `(T, error)` whenever any
    // param requires JSON marshalling, even when the underlying Rust function
    // does not return Result. Detect that so error-expecting tests emit `_, err :=`
    // instead of `err :=` when the binding has a value component.
    let binding_returns_error_pre = args
        .iter()
        .any(|a| matches!(a.arg_type.as_str(), "json_object" | "bytes"));
    let effective_returns_result_pre = returns_result || binding_returns_error_pre || client_factory.is_some();

    if expects_error {
        if effective_returns_result_pre && !returns_void {
            let _ = writeln!(out, "\t_, err := {call_prefix}.{function_name}({final_args})");
        } else {
            let _ = writeln!(out, "\terr := {call_prefix}.{function_name}({final_args})");
        }
        let _ = writeln!(out, "\tif err == nil {{");
        let _ = writeln!(out, "\t\tt.Errorf(\"expected an error, but call succeeded\")");
        let _ = writeln!(out, "\t}}");
        let _ = writeln!(out, "}}");
        return;
    }

    // Check if any assertion actually uses the result variable.
    // If all assertions are skipped (field not on result type), use `_` to avoid
    // Go's "declared and not used" compile error.
    let has_usable_assertion = fixture.assertions.iter().any(|a| {
        if a.assertion_type == "not_error" || a.assertion_type == "error" {
            return false;
        }
        // method_result assertions always use the result variable.
        if a.assertion_type == "method_result" {
            return true;
        }
        match &a.field {
            Some(f) if !f.is_empty() => field_resolver.is_valid_for_result(f),
            _ => true,
        }
    });

    // The Go binding generator (alef-backend-go) wraps the FFI call in `(T, error)`
    // whenever any param requires JSON marshalling (Vec, Map, Named struct), even when
    // the underlying Rust function does not return Result. So a result_is_simple call
    // like `generate_cache_key(parts: &[(String, String)]) -> String` still surfaces in
    // Go as `func GenerateCacheKey(parts [][]string) (*string, error)`. Detect that
    // here so the test emits `_, err :=` / `result, err :=` instead of `result :=`.
    let binding_returns_error = args
        .iter()
        .any(|a| matches!(a.arg_type.as_str(), "json_object" | "bytes"));
    // Client-factory methods always return (value, error) in the Go binding.
    let effective_returns_result = returns_result || binding_returns_error || client_factory.is_some();

    // For result_is_simple functions, the result variable IS the value (e.g. *string, *bool).
    // We create a local `value` that dereferences it so assertions can use a plain type.
    // For functions that return (value, error): emit `result, err :=`
    // For functions that return only error: emit `err :=`
    // For functions that return only a value (result_is_simple, no error): emit `result :=`
    if !effective_returns_result && result_is_simple {
        // Function returns a single value, no error (e.g. *string, *bool).
        let result_binding = if has_usable_assertion {
            result_var.to_string()
        } else {
            "_".to_string()
        };
        // In Go, `_ :=` is invalid — must use `_ =` for the blank identifier.
        let assign_op = if result_binding == "_" { "=" } else { ":=" };
        let _ = writeln!(
            out,
            "\t{result_binding} {assign_op} {call_prefix}.{function_name}({final_args})"
        );
        if has_usable_assertion && result_binding != "_" {
            if result_is_array {
                // Array results are slices (not pointers); assign directly without dereference.
                let _ = writeln!(out, "\tvalue := {result_var}");
            } else {
                // Emit nil check and dereference for simple pointer results.
                let _ = writeln!(out, "\tif {result_var} == nil {{");
                let _ = writeln!(out, "\t\tt.Fatalf(\"expected non-nil result\")");
                let _ = writeln!(out, "\t}}");
                let _ = writeln!(out, "\tvalue := *{result_var}");
            }
        }
    } else if !effective_returns_result || returns_void {
        // Function returns only error (either returns_result=false, or returns_result=true
        // with returns_void=true meaning the Go function signature is `func(...) error`).
        let _ = writeln!(out, "\terr := {call_prefix}.{function_name}({final_args})");
        let _ = writeln!(out, "\tif err != nil {{");
        let _ = writeln!(out, "\t\tt.Fatalf(\"call failed: %v\", err)");
        let _ = writeln!(out, "\t}}");
        // No result variable to use in assertions.
        let _ = writeln!(out, "}}");
        return;
    } else {
        // returns_result = true, returns_void = false: function returns (value, error).
        let result_binding = if has_usable_assertion {
            result_var.to_string()
        } else {
            "_".to_string()
        };
        let _ = writeln!(
            out,
            "\t{result_binding}, err := {call_prefix}.{function_name}({final_args})"
        );
        let _ = writeln!(out, "\tif err != nil {{");
        let _ = writeln!(out, "\t\tt.Fatalf(\"call failed: %v\", err)");
        let _ = writeln!(out, "\t}}");
        if result_is_simple && has_usable_assertion && result_binding != "_" {
            if result_is_array {
                // Array results are slices (not pointers); assign directly without dereference.
                let _ = writeln!(out, "\tvalue := {result_var}");
            } else {
                // Emit nil check and dereference for simple pointer results.
                let _ = writeln!(out, "\tif {result_var} == nil {{");
                let _ = writeln!(out, "\t\tt.Fatalf(\"expected non-nil result\")");
                let _ = writeln!(out, "\t}}");
                let _ = writeln!(out, "\tvalue := *{result_var}");
            }
        }
    }

    // For result_is_simple functions, assertions reference `value` (the dereferenced result).
    let effective_result_var = if result_is_simple && has_usable_assertion {
        "value".to_string()
    } else {
        result_var.to_string()
    };

    // Collect optional fields referenced by assertions and emit nil-safe
    // dereference blocks so that assertions can use plain string locals.
    // Only dereference fields whose assertion values are strings (or that are
    // used in string-oriented assertions like equals/contains with string values).
    let mut optional_locals: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for assertion in &fixture.assertions {
        if let Some(f) = &assertion.field {
            if !f.is_empty() {
                let resolved = field_resolver.resolve(f);
                if field_resolver.is_optional(resolved) && !optional_locals.contains_key(f.as_str()) {
                    // Only create deref locals for string-valued fields that are NOT arrays.
                    // Array fields (e.g., *[]string) must keep their pointer form so
                    // render_assertion can emit strings.Join(*field, " ") rather than
                    // treating them as plain strings.
                    let is_string_field = assertion.value.as_ref().is_some_and(|v| v.is_string());
                    let is_array_field = field_resolver.is_array(resolved);
                    if !is_string_field || is_array_field {
                        // Non-string optional fields (e.g., *uint64) and array optional
                        // fields (e.g., *[]string) are handled by nil guards in render_assertion.
                        continue;
                    }
                    let field_expr = field_resolver.accessor(f, "go", &effective_result_var);
                    let local_var = go_param_name(&resolved.replace(['.', '[', ']'], "_"));
                    if field_resolver.has_map_access(f) {
                        // Go map access returns a value type (string), not a pointer.
                        // Use the value directly — empty string means not present.
                        let _ = writeln!(out, "\t{local_var} := {field_expr}");
                    } else {
                        let _ = writeln!(out, "\tvar {local_var} string");
                        let _ = writeln!(out, "\tif {field_expr} != nil {{");
                        let _ = writeln!(out, "\t\t{local_var} = *{field_expr}");
                        let _ = writeln!(out, "\t}}");
                    }
                    optional_locals.insert(f.clone(), local_var);
                }
            }
        }
    }

    // Emit assertions, wrapping in nil guards when an intermediate path segment is optional.
    for assertion in &fixture.assertions {
        if let Some(f) = &assertion.field {
            if !f.is_empty() && !optional_locals.contains_key(f.as_str()) {
                // Check if any prefix of the dotted path is optional (pointer in Go).
                // e.g., "document.nodes" — if "document" is optional, guard the whole block.
                let parts: Vec<&str> = f.split('.').collect();
                let mut guard_expr: Option<String> = None;
                for i in 1..parts.len() {
                    let prefix = parts[..i].join(".");
                    let resolved_prefix = field_resolver.resolve(&prefix);
                    if field_resolver.is_optional(resolved_prefix) {
                        let accessor = field_resolver.accessor(&prefix, "go", &effective_result_var);
                        guard_expr = Some(accessor);
                        break;
                    }
                }
                if let Some(guard) = guard_expr {
                    // Only emit nil guard if the assertion will actually produce code
                    // (not just a skip comment), to avoid empty branches (SA9003).
                    if field_resolver.is_valid_for_result(f) {
                        let _ = writeln!(out, "\tif {guard} != nil {{");
                        // Render into a temporary buffer so we can re-indent by one
                        // tab level to sit inside the nil-guard block.
                        let mut nil_buf = String::new();
                        render_assertion(
                            &mut nil_buf,
                            assertion,
                            &effective_result_var,
                            import_alias,
                            field_resolver,
                            &optional_locals,
                            result_is_simple,
                            result_is_array,
                        );
                        for line in nil_buf.lines() {
                            let _ = writeln!(out, "\t{line}");
                        }
                        let _ = writeln!(out, "\t}}");
                    } else {
                        render_assertion(
                            out,
                            assertion,
                            &effective_result_var,
                            import_alias,
                            field_resolver,
                            &optional_locals,
                            result_is_simple,
                            result_is_array,
                        );
                    }
                    continue;
                }
            }
        }
        render_assertion(
            out,
            assertion,
            &effective_result_var,
            import_alias,
            field_resolver,
            &optional_locals,
            result_is_simple,
            result_is_array,
        );
    }

    let _ = writeln!(out, "}}");
}

/// Render an HTTP server test function using net/http against MOCK_SERVER_URL.
///
/// Delegates to the shared driver [`client::http_call::render_http_test`] via
/// [`GoTestClientRenderer`]. The emitted test shape is unchanged: `func Test_<Name>(t *testing.T)`
/// with a `net/http` client that hits `$MOCK_SERVER_URL/fixtures/<id>`.
fn render_http_test_function(out: &mut String, fixture: &Fixture) {
    client::http_call::render_http_test(out, &GoTestClientRenderer, fixture);
}

// ---------------------------------------------------------------------------
// HTTP test rendering — GoTestClientRenderer
// ---------------------------------------------------------------------------

/// Go `net/http` test renderer.
///
/// Go HTTP e2e tests send a request to `$MOCK_SERVER_URL/fixtures/<id>` using
/// the standard library `net/http` client. The trait primitives emit the
/// request-build, response-capture, and assertion code that the previous
/// monolithic renderer produced, so generated output is unchanged after the
/// migration.
struct GoTestClientRenderer;

impl client::TestClientRenderer for GoTestClientRenderer {
    fn language_name(&self) -> &'static str {
        "go"
    }

    /// Go test names use `UpperCamelCase` so they form valid exported identifiers
    /// (e.g. `Test_MyFixtureId`). Override the default `sanitize_ident` which
    /// produces `lower_snake_case`.
    fn sanitize_test_name(&self, id: &str) -> String {
        id.to_upper_camel_case()
    }

    /// Emit `func Test_<fn_name>(t *testing.T) {`, a description comment, and the
    /// `baseURL` / request scaffolding. Skipped fixtures get `t.Skip(...)` inline.
    fn render_test_open(&self, out: &mut String, fn_name: &str, description: &str, skip_reason: Option<&str>) {
        let _ = writeln!(out, "func Test_{fn_name}(t *testing.T) {{");
        let _ = writeln!(out, "\t// {description}");
        if let Some(reason) = skip_reason {
            let escaped = go_string_literal(reason);
            let _ = writeln!(out, "\tt.Skip({escaped})");
        }
    }

    fn render_test_close(&self, out: &mut String) {
        let _ = writeln!(out, "}}");
    }

    /// Emit the full `net/http` request scaffolding: URL construction, body,
    /// headers, cookies, a no-redirect client, and `io.ReadAll` for the body.
    ///
    /// `bodyBytes` is always declared (with `_ = bodyBytes` to avoid the Go
    /// "declared and not used" compile error on tests with no body assertion).
    fn render_call(&self, out: &mut String, ctx: &client::CallCtx<'_>) {
        let method = ctx.method.to_uppercase();
        let path = ctx.path;

        let _ = writeln!(out, "\tbaseURL := os.Getenv(\"MOCK_SERVER_URL\")");
        let _ = writeln!(out, "\tif baseURL == \"\" {{");
        let _ = writeln!(out, "\t\tbaseURL = \"http://localhost:8080\"");
        let _ = writeln!(out, "\t}}");

        // Build request body expression.
        let body_expr = if let Some(body) = ctx.body {
            let json = serde_json::to_string(body).unwrap_or_default();
            let escaped = go_string_literal(&json);
            format!("strings.NewReader({})", escaped)
        } else {
            "strings.NewReader(\"\")".to_string()
        };

        let _ = writeln!(out, "\tbody := {body_expr}");
        let _ = writeln!(
            out,
            "\treq, err := http.NewRequest(\"{method}\", baseURL+\"{path}\", body)"
        );
        let _ = writeln!(out, "\tif err != nil {{");
        let _ = writeln!(out, "\t\tt.Fatalf(\"new request failed: %v\", err)");
        let _ = writeln!(out, "\t}}");

        // Content-Type header (only when a body is present).
        if ctx.body.is_some() {
            let content_type = ctx.content_type.unwrap_or("application/json");
            let _ = writeln!(out, "\treq.Header.Set(\"Content-Type\", \"{content_type}\")");
        }

        // Explicit request headers (sorted for deterministic output).
        let mut header_names: Vec<&String> = ctx.headers.keys().collect();
        header_names.sort();
        for name in header_names {
            let value = &ctx.headers[name];
            let escaped_name = go_string_literal(name);
            let escaped_value = go_string_literal(value);
            let _ = writeln!(out, "\treq.Header.Set({escaped_name}, {escaped_value})");
        }

        // Cookies.
        if !ctx.cookies.is_empty() {
            let mut cookie_names: Vec<&String> = ctx.cookies.keys().collect();
            cookie_names.sort();
            for name in cookie_names {
                let value = &ctx.cookies[name];
                let escaped_name = go_string_literal(name);
                let escaped_value = go_string_literal(value);
                let _ = writeln!(
                    out,
                    "\treq.AddCookie(&http.Cookie{{Name: {escaped_name}, Value: {escaped_value}}})"
                );
            }
        }

        // No-redirect client so 3xx fixtures assert the redirect response itself.
        let _ = writeln!(out, "\tnoRedirectClient := &http.Client{{");
        let _ = writeln!(
            out,
            "\t\tCheckRedirect: func(req *http.Request, via []*http.Request) error {{"
        );
        let _ = writeln!(out, "\t\t\treturn http.ErrUseLastResponse");
        let _ = writeln!(out, "\t\t}},");
        let _ = writeln!(out, "\t}}");
        let _ = writeln!(out, "\tresp, err := noRedirectClient.Do(req)");
        let _ = writeln!(out, "\tif err != nil {{");
        let _ = writeln!(out, "\t\tt.Fatalf(\"request failed: %v\", err)");
        let _ = writeln!(out, "\t}}");
        let _ = writeln!(out, "\tdefer resp.Body.Close()");

        // Always read the response body so body-assertion methods can reference
        // `bodyBytes`. Suppress the "declared and not used" compile error with
        // `_ = bodyBytes` for tests that have no body assertion.
        let _ = writeln!(out, "\tbodyBytes, err := io.ReadAll(resp.Body)");
        let _ = writeln!(out, "\tif err != nil {{");
        let _ = writeln!(out, "\t\tt.Fatalf(\"read body failed: %v\", err)");
        let _ = writeln!(out, "\t}}");
        let _ = writeln!(out, "\t_ = bodyBytes");
    }

    fn render_assert_status(&self, out: &mut String, _response_var: &str, status: u16) {
        let _ = writeln!(out, "\tif resp.StatusCode != {status} {{");
        let _ = writeln!(out, "\t\tt.Fatalf(\"status: got %d want {status}\", resp.StatusCode)");
        let _ = writeln!(out, "\t}}");
    }

    /// Emit a header assertion, skipping special tokens (`<<present>>`, `<<absent>>`,
    /// `<<uuid>>`) and hop-by-hop headers (`Connection`) that `net/http` strips.
    fn render_assert_header(&self, out: &mut String, _response_var: &str, name: &str, expected: &str) {
        // Skip special-token assertions.
        if matches!(expected, "<<absent>>" | "<<present>>" | "<<uuid>>") {
            return;
        }
        // Connection is a hop-by-hop header that Go's net/http strips.
        if name.eq_ignore_ascii_case("connection") {
            return;
        }
        let escaped_name = go_string_literal(name);
        let escaped_value = go_string_literal(expected);
        let _ = writeln!(
            out,
            "\tif !strings.Contains(resp.Header.Get({escaped_name}), {escaped_value}) {{"
        );
        let _ = writeln!(
            out,
            "\t\tt.Fatalf(\"header %s mismatch: got %q want to contain %q\", {escaped_name}, resp.Header.Get({escaped_name}), {escaped_value})"
        );
        let _ = writeln!(out, "\t}}");
    }

    /// Emit an exact-equality body assertion.
    ///
    /// JSON objects and arrays are round-tripped via `json.Unmarshal` + `reflect.DeepEqual`.
    /// Scalar values are compared as trimmed strings.
    fn render_assert_json_body(&self, out: &mut String, _response_var: &str, expected: &serde_json::Value) {
        match expected {
            serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
                let json_str = serde_json::to_string(expected).unwrap_or_default();
                let escaped = go_string_literal(&json_str);
                let _ = writeln!(out, "\tvar got any");
                let _ = writeln!(out, "\tvar want any");
                let _ = writeln!(out, "\tif err := json.Unmarshal(bodyBytes, &got); err != nil {{");
                let _ = writeln!(out, "\t\tt.Fatalf(\"json unmarshal got: %v\", err)");
                let _ = writeln!(out, "\t}}");
                let _ = writeln!(
                    out,
                    "\tif err := json.Unmarshal([]byte({escaped}), &want); err != nil {{"
                );
                let _ = writeln!(out, "\t\tt.Fatalf(\"json unmarshal want: %v\", err)");
                let _ = writeln!(out, "\t}}");
                let _ = writeln!(out, "\tif !reflect.DeepEqual(got, want) {{");
                let _ = writeln!(out, "\t\tt.Fatalf(\"body mismatch: got %v want %v\", got, want)");
                let _ = writeln!(out, "\t}}");
            }
            serde_json::Value::String(s) => {
                let escaped = go_string_literal(s);
                let _ = writeln!(out, "\twant := {escaped}");
                let _ = writeln!(out, "\tif strings.TrimSpace(string(bodyBytes)) != want {{");
                let _ = writeln!(out, "\t\tt.Fatalf(\"body: got %q want %q\", string(bodyBytes), want)");
                let _ = writeln!(out, "\t}}");
            }
            other => {
                let escaped = go_string_literal(&other.to_string());
                let _ = writeln!(out, "\twant := {escaped}");
                let _ = writeln!(out, "\tif strings.TrimSpace(string(bodyBytes)) != want {{");
                let _ = writeln!(out, "\t\tt.Fatalf(\"body: got %q want %q\", string(bodyBytes), want)");
                let _ = writeln!(out, "\t}}");
            }
        }
    }

    /// Emit partial-body assertions: every key in `expected` must appear in the
    /// parsed JSON response with the matching value.
    fn render_assert_partial_body(&self, out: &mut String, _response_var: &str, expected: &serde_json::Value) {
        if let Some(obj) = expected.as_object() {
            let _ = writeln!(out, "\tvar _partialGot map[string]any");
            let _ = writeln!(
                out,
                "\tif err := json.Unmarshal(bodyBytes, &_partialGot); err != nil {{"
            );
            let _ = writeln!(out, "\t\tt.Fatalf(\"json unmarshal partial: %v\", err)");
            let _ = writeln!(out, "\t}}");
            for (key, val) in obj {
                let escaped_key = go_string_literal(key);
                let json_val = serde_json::to_string(val).unwrap_or_default();
                let escaped_val = go_string_literal(&json_val);
                let _ = writeln!(out, "\t{{");
                let _ = writeln!(out, "\t\tvar _wantVal any");
                let _ = writeln!(
                    out,
                    "\t\tif err := json.Unmarshal([]byte({escaped_val}), &_wantVal); err != nil {{"
                );
                let _ = writeln!(out, "\t\t\tt.Fatalf(\"json unmarshal partial want: %v\", err)");
                let _ = writeln!(out, "\t\t}}");
                let _ = writeln!(
                    out,
                    "\t\tif !reflect.DeepEqual(_partialGot[{escaped_key}], _wantVal) {{"
                );
                let _ = writeln!(
                    out,
                    "\t\t\tt.Fatalf(\"partial body field {key}: got %v want %v\", _partialGot[{escaped_key}], _wantVal)"
                );
                let _ = writeln!(out, "\t\t}}");
                let _ = writeln!(out, "\t}}");
            }
        }
    }

    /// Emit validation-error assertions for 422 responses.
    ///
    /// Checks that each expected `msg` appears in at least one element of the
    /// parsed body's `"errors"` array.
    fn render_assert_validation_errors(
        &self,
        out: &mut String,
        _response_var: &str,
        errors: &[ValidationErrorExpectation],
    ) {
        let _ = writeln!(out, "\tvar _veBody map[string]any");
        let _ = writeln!(out, "\tif err := json.Unmarshal(bodyBytes, &_veBody); err != nil {{");
        let _ = writeln!(out, "\t\tt.Fatalf(\"json unmarshal validation errors: %v\", err)");
        let _ = writeln!(out, "\t}}");
        let _ = writeln!(out, "\t_veErrors, _ := _veBody[\"errors\"].([]any)");
        for ve in errors {
            let escaped_msg = go_string_literal(&ve.msg);
            let _ = writeln!(out, "\t{{");
            let _ = writeln!(out, "\t\t_found := false");
            let _ = writeln!(out, "\t\tfor _, _e := range _veErrors {{");
            let _ = writeln!(out, "\t\t\tif _em, ok := _e.(map[string]any); ok {{");
            let _ = writeln!(
                out,
                "\t\t\t\tif _msg, ok := _em[\"msg\"].(string); ok && strings.Contains(_msg, {escaped_msg}) {{"
            );
            let _ = writeln!(out, "\t\t\t\t\t_found = true");
            let _ = writeln!(out, "\t\t\t\t\tbreak");
            let _ = writeln!(out, "\t\t\t\t}}");
            let _ = writeln!(out, "\t\t\t}}");
            let _ = writeln!(out, "\t\t}}");
            let _ = writeln!(out, "\t\tif !_found {{");
            let _ = writeln!(
                out,
                "\t\t\tt.Fatalf(\"validation error with msg containing %q not found in errors\", {escaped_msg})"
            );
            let _ = writeln!(out, "\t\t}}");
            let _ = writeln!(out, "\t}}");
        }
    }
}

/// Build setup lines (e.g. handle creation) and the argument list for the function call.
///
/// Returns `(setup_lines, args_string)`.
///
/// `options_ptr` — when `true`, `json_object` args with an `options_type` are
/// passed as a Go pointer (`*OptionsType`): absent/empty → `nil`, present →
/// `&varName` after JSON unmarshal.
fn build_args_and_setup(
    input: &serde_json::Value,
    args: &[crate::config::ArgMapping],
    import_alias: &str,
    options_type: Option<&str>,
    fixture_id: &str,
    options_ptr: bool,
) -> (Vec<String>, String) {
    use heck::ToUpperCamelCase;

    if args.is_empty() {
        return (Vec::new(), String::new());
    }

    let mut setup_lines: Vec<String> = Vec::new();
    let mut parts: Vec<String> = Vec::new();

    for arg in args {
        if arg.arg_type == "mock_url" {
            setup_lines.push(format!(
                "{} := os.Getenv(\"MOCK_SERVER_URL\") + \"/fixtures/{fixture_id}\"",
                arg.name,
            ));
            parts.push(arg.name.clone());
            continue;
        }

        if arg.arg_type == "handle" {
            // Generate a CreateEngine (or equivalent) call and pass the variable.
            let constructor_name = format!("Create{}", arg.name.to_upper_camel_case());
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let config_value = input.get(field).unwrap_or(&serde_json::Value::Null);
            if config_value.is_null()
                || config_value.is_object() && config_value.as_object().is_some_and(|o| o.is_empty())
            {
                setup_lines.push(format!(
                    "{name}, createErr := {import_alias}.{constructor_name}(nil)\n\tif createErr != nil {{\n\t\tt.Fatalf(\"create handle failed: %v\", createErr)\n\t}}",
                    name = arg.name,
                ));
            } else {
                let json_str = serde_json::to_string(config_value).unwrap_or_default();
                let go_literal = go_string_literal(&json_str);
                let name = &arg.name;
                setup_lines.push(format!(
                    "var {name}Config {import_alias}.CrawlConfig\n\tif err := json.Unmarshal([]byte({go_literal}), &{name}Config); err != nil {{\n\t\tt.Fatalf(\"config parse failed: %v\", err)\n\t}}"
                ));
                setup_lines.push(format!(
                    "{name}, createErr := {import_alias}.{constructor_name}(&{name}Config)\n\tif createErr != nil {{\n\t\tt.Fatalf(\"create handle failed: %v\", createErr)\n\t}}"
                ));
            }
            parts.push(arg.name.clone());
            continue;
        }

        let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
        let val = input.get(field);

        // Handle bytes type: fixture stores base64-encoded bytes.
        // Emit a Go base64.StdEncoding.DecodeString call to decode at runtime.
        if arg.arg_type == "bytes" {
            let var_name = format!("{}Bytes", arg.name);
            match val {
                None | Some(serde_json::Value::Null) => {
                    if arg.optional {
                        parts.push("nil".to_string());
                    } else {
                        parts.push("[]byte{}".to_string());
                    }
                }
                Some(serde_json::Value::String(s)) => {
                    let go_b64 = go_string_literal(s);
                    setup_lines.push(format!("{var_name}, _ := base64.StdEncoding.DecodeString({go_b64})"));
                    parts.push(var_name);
                }
                Some(other) => {
                    parts.push(format!("[]byte({})", json_to_go(other)));
                }
            }
            continue;
        }

        match val {
            None | Some(serde_json::Value::Null) if arg.optional => {
                // Optional arg absent: emit Go zero/nil for the type.
                match arg.arg_type.as_str() {
                    "string" => {
                        // Optional string in Go bindings is *string → nil.
                        parts.push("nil".to_string());
                    }
                    "json_object" => {
                        if options_ptr {
                            // Pointer options type (*OptionsType): absent → nil.
                            parts.push("nil".to_string());
                        } else if let Some(opts_type) = options_type {
                            // Value options type: zero-value struct.
                            parts.push(format!("{import_alias}.{opts_type}{{}}"));
                        } else {
                            parts.push("nil".to_string());
                        }
                    }
                    _ => {
                        parts.push("nil".to_string());
                    }
                }
            }
            None | Some(serde_json::Value::Null) => {
                // Required arg with no fixture value: pass a language-appropriate default.
                let default_val = match arg.arg_type.as_str() {
                    "string" => "\"\"".to_string(),
                    "int" | "integer" | "i64" => "0".to_string(),
                    "float" | "number" => "0.0".to_string(),
                    "bool" | "boolean" => "false".to_string(),
                    "json_object" => {
                        if options_ptr {
                            // Pointer options type (*OptionsType): absent → nil.
                            "nil".to_string()
                        } else if let Some(opts_type) = options_type {
                            format!("{import_alias}.{opts_type}{{}}")
                        } else {
                            "nil".to_string()
                        }
                    }
                    _ => "nil".to_string(),
                };
                parts.push(default_val);
            }
            Some(v) => {
                match arg.arg_type.as_str() {
                    "json_object" => {
                        // JSON arrays unmarshal into []string (Go slices).
                        // JSON objects with a known options_type unmarshal into that type.
                        let is_array = v.is_array();
                        let is_empty_obj = !is_array && v.is_object() && v.as_object().is_some_and(|o| o.is_empty());
                        if is_empty_obj {
                            if options_ptr {
                                // Pointer options type: empty object → nil.
                                parts.push("nil".to_string());
                            } else if let Some(opts_type) = options_type {
                                parts.push(format!("{import_alias}.{opts_type}{{}}"));
                            } else {
                                parts.push("nil".to_string());
                            }
                        } else if is_array {
                            // Array type — unmarshal into a Go slice. Honor `go_type` for a
                            // fully explicit Go type (e.g. `"kreuzberg.BatchBytesItem"`), fall
                            // back to deriving the slice type from `element_type`, defaulting
                            // to `[]string` for unknown types.
                            let go_slice_type = if let Some(go_t) = arg.go_type.as_deref() {
                                // go_type is the slice element type — wrap it in [].
                                // If it already starts with '[' the user specified the full
                                // slice type; use it verbatim.
                                if go_t.starts_with('[') {
                                    go_t.to_string()
                                } else {
                                    // Qualify unqualified types (e.g., "BatchBytesItem" → "kreuzberg.BatchBytesItem")
                                    let qualified = if go_t.contains('.') {
                                        go_t.to_string()
                                    } else {
                                        format!("{import_alias}.{go_t}")
                                    };
                                    format!("[]{qualified}")
                                }
                            } else {
                                element_type_to_go_slice(arg.element_type.as_deref(), import_alias)
                            };
                            let json_str = serde_json::to_string(v).unwrap_or_default();
                            let go_literal = go_string_literal(&json_str);
                            let var_name = &arg.name;
                            setup_lines.push(format!(
                                "var {var_name} {go_slice_type}\n\tif err := json.Unmarshal([]byte({go_literal}), &{var_name}); err != nil {{\n\t\tt.Fatalf(\"config parse failed: %v\", err)\n\t}}"
                            ));
                            parts.push(var_name.to_string());
                        } else if let Some(opts_type) = options_type {
                            // Object with known type — unmarshal into typed struct.
                            // When options_ptr is set, the Go struct uses snake_case JSON
                            // field tags and lowercase/snake_case enum values.  Remap the
                            // fixture's camelCase keys and PascalCase enum string values.
                            let remapped_v = if options_ptr {
                                convert_json_for_go(v.clone())
                            } else {
                                v.clone()
                            };
                            let json_str = serde_json::to_string(&remapped_v).unwrap_or_default();
                            let go_literal = go_string_literal(&json_str);
                            let var_name = &arg.name;
                            setup_lines.push(format!(
                                "var {var_name} {import_alias}.{opts_type}\n\tif err := json.Unmarshal([]byte({go_literal}), &{var_name}); err != nil {{\n\t\tt.Fatalf(\"config parse failed: %v\", err)\n\t}}"
                            ));
                            // Pass as pointer when options_ptr is set.
                            let arg_expr = if options_ptr {
                                format!("&{var_name}")
                            } else {
                                var_name.to_string()
                            };
                            parts.push(arg_expr);
                        } else {
                            parts.push(json_to_go(v));
                        }
                    }
                    "string" if arg.optional => {
                        // Optional string in Go is *string — take address of a local.
                        let var_name = format!("{}Val", arg.name);
                        let go_val = json_to_go(v);
                        setup_lines.push(format!("{var_name} := {go_val}"));
                        parts.push(format!("&{var_name}"));
                    }
                    _ => {
                        parts.push(json_to_go(v));
                    }
                }
            }
        }
    }

    (setup_lines, parts.join(", "))
}

#[allow(clippy::too_many_arguments)]
fn render_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    import_alias: &str,
    field_resolver: &FieldResolver,
    optional_locals: &std::collections::HashMap<String, String>,
    result_is_simple: bool,
    result_is_array: bool,
) {
    // Handle synthetic / derived fields before the is_valid_for_result check
    // so they are never treated as struct field accesses on the result.
    if !result_is_simple {
        if let Some(f) = &assertion.field {
            // embed_texts returns *[][]float32; the embedding matrix is *result_var.
            // We emit inline func() expressions so we don't need additional variables.
            let embed_deref = format!("(*{result_var})");
            match f.as_str() {
                "chunks_have_content" => {
                    let pred = format!(
                        "func() bool {{ chunks := {result_var}.Chunks; if chunks == nil {{ return false }}; for _, c := range *chunks {{ if c.Content == \"\" {{ return false }} }}; return true }}()"
                    );
                    match assertion.assertion_type.as_str() {
                        "is_true" => {
                            let _ = writeln!(out, "\tassert.True(t, {pred}, \"expected true\")");
                        }
                        "is_false" => {
                            let _ = writeln!(out, "\tassert.False(t, {pred}, \"expected false\")");
                        }
                        _ => {
                            let _ = writeln!(out, "\t// skipped: unsupported assertion type on synthetic field '{f}'");
                        }
                    }
                    return;
                }
                "chunks_have_embeddings" => {
                    let pred = format!(
                        "func() bool {{ chunks := {result_var}.Chunks; if chunks == nil {{ return false }}; for _, c := range *chunks {{ if c.Embedding == nil || len(*c.Embedding) == 0 {{ return false }} }}; return true }}()"
                    );
                    match assertion.assertion_type.as_str() {
                        "is_true" => {
                            let _ = writeln!(out, "\tassert.True(t, {pred}, \"expected true\")");
                        }
                        "is_false" => {
                            let _ = writeln!(out, "\tassert.False(t, {pred}, \"expected false\")");
                        }
                        _ => {
                            let _ = writeln!(out, "\t// skipped: unsupported assertion type on synthetic field '{f}'");
                        }
                    }
                    return;
                }
                "embeddings" => {
                    match assertion.assertion_type.as_str() {
                        "count_equals" => {
                            if let Some(val) = &assertion.value {
                                if let Some(n) = val.as_u64() {
                                    let _ = writeln!(
                                        out,
                                        "\tassert.Equal(t, {n}, len({embed_deref}), \"expected exactly {n} elements\")"
                                    );
                                }
                            }
                        }
                        "count_min" => {
                            if let Some(val) = &assertion.value {
                                if let Some(n) = val.as_u64() {
                                    let _ = writeln!(
                                        out,
                                        "\tassert.GreaterOrEqual(t, len({embed_deref}), {n}, \"expected at least {n} elements\")"
                                    );
                                }
                            }
                        }
                        "not_empty" => {
                            let _ = writeln!(
                                out,
                                "\tassert.NotEmpty(t, {embed_deref}, \"expected non-empty embeddings\")"
                            );
                        }
                        "is_empty" => {
                            let _ = writeln!(out, "\tassert.Empty(t, {embed_deref}, \"expected empty embeddings\")");
                        }
                        _ => {
                            let _ = writeln!(
                                out,
                                "\t// skipped: unsupported assertion type on synthetic field 'embeddings'"
                            );
                        }
                    }
                    return;
                }
                "embedding_dimensions" => {
                    let expr = format!(
                        "func() int {{ if len({embed_deref}) == 0 {{ return 0 }}; return len({embed_deref}[0]) }}()"
                    );
                    match assertion.assertion_type.as_str() {
                        "equals" => {
                            if let Some(val) = &assertion.value {
                                if let Some(n) = val.as_u64() {
                                    let _ = writeln!(
                                        out,
                                        "\tif {expr} != {n} {{\n\t\tt.Errorf(\"equals mismatch: got %v\", {expr})\n\t}}"
                                    );
                                }
                            }
                        }
                        "greater_than" => {
                            if let Some(val) = &assertion.value {
                                if let Some(n) = val.as_u64() {
                                    let _ = writeln!(out, "\tassert.Greater(t, {expr}, {n}, \"expected > {n}\")");
                                }
                            }
                        }
                        _ => {
                            let _ = writeln!(
                                out,
                                "\t// skipped: unsupported assertion type on synthetic field 'embedding_dimensions'"
                            );
                        }
                    }
                    return;
                }
                "embeddings_valid" | "embeddings_finite" | "embeddings_non_zero" | "embeddings_normalized" => {
                    let pred = match f.as_str() {
                        "embeddings_valid" => {
                            format!(
                                "func() bool {{ for _, e := range {embed_deref} {{ if len(e) == 0 {{ return false }} }}; return true }}()"
                            )
                        }
                        "embeddings_finite" => {
                            format!(
                                "func() bool {{ for _, e := range {embed_deref} {{ for _, v := range e {{ if v != v || v == float32(1.0/0.0) || v == float32(-1.0/0.0) {{ return false }} }} }}; return true }}()"
                            )
                        }
                        "embeddings_non_zero" => {
                            format!(
                                "func() bool {{ for _, e := range {embed_deref} {{ hasNonZero := false; for _, v := range e {{ if v != 0 {{ hasNonZero = true; break }} }}; if !hasNonZero {{ return false }} }}; return true }}()"
                            )
                        }
                        "embeddings_normalized" => {
                            format!(
                                "func() bool {{ for _, e := range {embed_deref} {{ var n float64; for _, v := range e {{ n += float64(v) * float64(v) }}; if n < 0.999 || n > 1.001 {{ return false }} }}; return true }}()"
                            )
                        }
                        _ => unreachable!(),
                    };
                    match assertion.assertion_type.as_str() {
                        "is_true" => {
                            let _ = writeln!(out, "\tassert.True(t, {pred}, \"expected true\")");
                        }
                        "is_false" => {
                            let _ = writeln!(out, "\tassert.False(t, {pred}, \"expected false\")");
                        }
                        _ => {
                            let _ = writeln!(out, "\t// skipped: unsupported assertion type on synthetic field '{f}'");
                        }
                    }
                    return;
                }
                // ---- keywords / keywords_count ----
                // Go ExtractionResult does not expose extracted_keywords; skip.
                "keywords" | "keywords_count" => {
                    let _ = writeln!(out, "\t// skipped: field '{f}' not available on Go ExtractionResult");
                    return;
                }
                _ => {}
            }
        }
    }

    // Skip assertions on fields that don't exist on the result type.
    // When result_is_simple, all field assertions operate on the scalar result directly.
    if !result_is_simple {
        if let Some(f) = &assertion.field {
            if !f.is_empty() && !field_resolver.is_valid_for_result(f) {
                let _ = writeln!(out, "\t// skipped: field '{f}' not available on result type");
                return;
            }
        }
    }

    let field_expr = if result_is_simple {
        // The result IS the value — field access is irrelevant.
        result_var.to_string()
    } else {
        match &assertion.field {
            Some(f) if !f.is_empty() => {
                // Use the local variable if the field was dereferenced above.
                if let Some(local_var) = optional_locals.get(f.as_str()) {
                    local_var.clone()
                } else {
                    field_resolver.accessor(f, "go", result_var)
                }
            }
            _ => result_var.to_string(),
        }
    };

    // Check if the field (after resolution) is optional, which means it's a pointer in Go.
    // Also check if a `.length` suffix's parent is optional (e.g., metadata.headings.length
    // where metadata.headings is optional → len() needs dereference).
    let is_optional = assertion
        .field
        .as_ref()
        .map(|f| {
            let resolved = field_resolver.resolve(f);
            let check_path = resolved
                .strip_suffix(".length")
                .or_else(|| resolved.strip_suffix(".count"))
                .or_else(|| resolved.strip_suffix(".size"))
                .unwrap_or(resolved);
            field_resolver.is_optional(check_path) && !optional_locals.contains_key(f.as_str())
        })
        .unwrap_or(false);

    // When field_expr is `len(X)` and X is an optional (pointer) field, rewrite to `len(*X)`
    // and we'll wrap with a nil guard in the assertion handlers.
    // However, slices are already nil-able and should not be dereferenced.
    let field_is_array_for_len = assertion
        .field
        .as_ref()
        .map(|f| {
            let resolved = field_resolver.resolve(f);
            let check_path = resolved
                .strip_suffix(".length")
                .or_else(|| resolved.strip_suffix(".count"))
                .or_else(|| resolved.strip_suffix(".size"))
                .unwrap_or(resolved);
            field_resolver.is_array(check_path)
        })
        .unwrap_or(false);
    let field_expr =
        if is_optional && field_expr.starts_with("len(") && field_expr.ends_with(')') && !field_is_array_for_len {
            let inner = &field_expr[4..field_expr.len() - 1];
            format!("len(*{inner})")
        } else {
            field_expr
        };
    // Build the nil-guard expression for the inner pointer (without len wrapper).
    let nil_guard_expr = if is_optional && field_expr.starts_with("len(*") {
        Some(field_expr[5..field_expr.len() - 1].to_string())
    } else {
        None
    };

    // For optional non-string fields that weren't dereferenced into locals,
    // we need to dereference the pointer in comparisons.
    // However, slices are already nil-able and should not be dereferenced.
    let field_is_slice = assertion
        .field
        .as_ref()
        .map(|f| field_resolver.is_array(field_resolver.resolve(f)))
        .unwrap_or(false);
    let deref_field_expr = if is_optional && !field_expr.starts_with("len(") && !field_is_slice {
        format!("*{field_expr}")
    } else {
        field_expr.clone()
    };

    // Detect array element access (e.g., `result.Assets[0].ContentHash`).
    // When the field_expr contains `[0]`, we must guard against an out-of-bounds
    // panic by checking that the array is non-empty first.
    // Extract the array slice expression (everything before `[0]`).
    let array_guard: Option<String> = if let Some(idx) = field_expr.find("[0]") {
        let mut array_expr = field_expr[..idx].to_string();
        if let Some(stripped) = array_expr.strip_prefix("len(") {
            array_expr = stripped.to_string();
        }
        Some(array_expr)
    } else {
        None
    };

    // Render the assertion into a temporary buffer first, then wrap with the array
    // bounds guard (if needed) by adding one extra level of indentation.
    let mut assertion_buf = String::new();
    let out_ref = &mut assertion_buf;

    match assertion.assertion_type.as_str() {
        "equals" => {
            if let Some(expected) = &assertion.value {
                let go_val = json_to_go(expected);
                // For string equality, trim whitespace to handle trailing newlines from the converter.
                if expected.is_string() {
                    // Wrap field expression with strings.TrimSpace() for string comparisons.
                    let trimmed_field = if is_optional && !field_expr.starts_with("len(") {
                        format!("strings.TrimSpace(*{field_expr})")
                    } else {
                        format!("strings.TrimSpace({field_expr})")
                    };
                    if is_optional && !field_expr.starts_with("len(") {
                        let _ = writeln!(out_ref, "\tif {field_expr} != nil && {trimmed_field} != {go_val} {{");
                    } else {
                        let _ = writeln!(out_ref, "\tif {trimmed_field} != {go_val} {{");
                    }
                } else if is_optional && !field_expr.starts_with("len(") {
                    let _ = writeln!(out_ref, "\tif {field_expr} != nil && {deref_field_expr} != {go_val} {{");
                } else {
                    let _ = writeln!(out_ref, "\tif {field_expr} != {go_val} {{");
                }
                let _ = writeln!(out_ref, "\t\tt.Errorf(\"equals mismatch: got %v\", {field_expr})");
                let _ = writeln!(out_ref, "\t}}");
            }
        }
        "contains" => {
            if let Some(expected) = &assertion.value {
                let go_val = json_to_go(expected);
                // Determine the "string view" of the field expression.
                // - *[]string → strings.Join(*field_expr, " ") for a nil-guarded check
                // - *string → string(*field_expr)
                // - string → string(field_expr) (or just field_expr for plain strings)
                // - result_is_array (result_is_simple + array result) → strings.Join(field_expr, " ")
                let resolved_field = assertion.field.as_deref().unwrap_or("");
                let resolved_name = field_resolver.resolve(resolved_field);
                let field_is_array = result_is_array || field_resolver.is_array(resolved_name);
                let is_opt =
                    is_optional && !optional_locals.contains_key(assertion.field.as_ref().unwrap_or(&String::new()));
                let field_for_contains = if is_opt && field_is_array {
                    format!("jsonString(*{field_expr})")
                } else if is_opt {
                    format!("fmt.Sprint(*{field_expr})")
                } else if field_is_array {
                    format!("jsonString({field_expr})")
                } else {
                    format!("fmt.Sprint({field_expr})")
                };
                if is_opt {
                    let _ = writeln!(out_ref, "\tif {field_expr} != nil {{");
                    let _ = writeln!(out_ref, "\tif !strings.Contains({field_for_contains}, {go_val}) {{");
                    let _ = writeln!(
                        out_ref,
                        "\t\tt.Errorf(\"expected to contain %s, got %v\", {go_val}, {field_expr})"
                    );
                    let _ = writeln!(out_ref, "\t}}");
                    let _ = writeln!(out_ref, "\t}}");
                } else {
                    let _ = writeln!(out_ref, "\tif !strings.Contains({field_for_contains}, {go_val}) {{");
                    let _ = writeln!(
                        out_ref,
                        "\t\tt.Errorf(\"expected to contain %s, got %v\", {go_val}, {field_expr})"
                    );
                    let _ = writeln!(out_ref, "\t}}");
                }
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                let resolved_field = assertion.field.as_deref().unwrap_or("");
                let resolved_name = field_resolver.resolve(resolved_field);
                let field_is_array = result_is_array || field_resolver.is_array(resolved_name);
                let is_opt =
                    is_optional && !optional_locals.contains_key(assertion.field.as_ref().unwrap_or(&String::new()));
                for val in values {
                    let go_val = json_to_go(val);
                    let field_for_contains = if is_opt && field_is_array {
                        format!("jsonString(*{field_expr})")
                    } else if is_opt {
                        format!("fmt.Sprint(*{field_expr})")
                    } else if field_is_array {
                        format!("jsonString({field_expr})")
                    } else {
                        format!("fmt.Sprint({field_expr})")
                    };
                    if is_opt {
                        let _ = writeln!(out_ref, "\tif {field_expr} != nil {{");
                        let _ = writeln!(out_ref, "\tif !strings.Contains({field_for_contains}, {go_val}) {{");
                        let _ = writeln!(out_ref, "\t\tt.Errorf(\"expected to contain %s\", {go_val})");
                        let _ = writeln!(out_ref, "\t}}");
                        let _ = writeln!(out_ref, "\t}}");
                    } else {
                        let _ = writeln!(out_ref, "\tif !strings.Contains({field_for_contains}, {go_val}) {{");
                        let _ = writeln!(out_ref, "\t\tt.Errorf(\"expected to contain %s\", {go_val})");
                        let _ = writeln!(out_ref, "\t}}");
                    }
                }
            }
        }
        "not_contains" => {
            if let Some(expected) = &assertion.value {
                let go_val = json_to_go(expected);
                let resolved_field = assertion.field.as_deref().unwrap_or("");
                let resolved_name = field_resolver.resolve(resolved_field);
                let field_is_array = result_is_array || field_resolver.is_array(resolved_name);
                let is_opt =
                    is_optional && !optional_locals.contains_key(assertion.field.as_ref().unwrap_or(&String::new()));
                let field_for_contains = if is_opt && field_is_array {
                    format!("jsonString(*{field_expr})")
                } else if is_opt {
                    format!("fmt.Sprint(*{field_expr})")
                } else if field_is_array {
                    format!("jsonString({field_expr})")
                } else {
                    format!("fmt.Sprint({field_expr})")
                };
                let _ = writeln!(out_ref, "\tif strings.Contains({field_for_contains}, {go_val}) {{");
                let _ = writeln!(
                    out_ref,
                    "\t\tt.Errorf(\"expected NOT to contain %s, got %v\", {go_val}, {field_expr})"
                );
                let _ = writeln!(out_ref, "\t}}");
            }
        }
        "not_empty" => {
            // For optional struct pointers (not arrays), just check != nil.
            // For optional slice/string pointers, check nil and len.
            let field_is_array = {
                let rf = assertion.field.as_deref().unwrap_or("");
                let rn = field_resolver.resolve(rf);
                field_resolver.is_array(rn)
            };
            if is_optional && !field_is_array {
                // Struct pointer: non-empty means not nil.
                let _ = writeln!(out_ref, "\tif {field_expr} == nil {{");
            } else if is_optional {
                let _ = writeln!(out_ref, "\tif {field_expr} == nil || len(*{field_expr}) == 0 {{");
            } else {
                let _ = writeln!(out_ref, "\tif len({field_expr}) == 0 {{");
            }
            let _ = writeln!(out_ref, "\t\tt.Errorf(\"expected non-empty value\")");
            let _ = writeln!(out_ref, "\t}}");
        }
        "is_empty" => {
            let field_is_array = {
                let rf = assertion.field.as_deref().unwrap_or("");
                let rn = field_resolver.resolve(rf);
                field_resolver.is_array(rn)
            };
            if is_optional && !field_is_array {
                // Struct pointer: empty means nil.
                let _ = writeln!(out_ref, "\tif {field_expr} != nil {{");
            } else if is_optional {
                let _ = writeln!(out_ref, "\tif {field_expr} != nil && len(*{field_expr}) != 0 {{");
            } else {
                let _ = writeln!(out_ref, "\tif len({field_expr}) != 0 {{");
            }
            let _ = writeln!(out_ref, "\t\tt.Errorf(\"expected empty value, got %v\", {field_expr})");
            let _ = writeln!(out_ref, "\t}}");
        }
        "contains_any" => {
            if let Some(values) = &assertion.values {
                let resolved_field = assertion.field.as_deref().unwrap_or("");
                let resolved_name = field_resolver.resolve(resolved_field);
                let field_is_array = field_resolver.is_array(resolved_name);
                let is_opt =
                    is_optional && !optional_locals.contains_key(assertion.field.as_ref().unwrap_or(&String::new()));
                let field_for_contains = if is_opt && field_is_array {
                    format!("jsonString(*{field_expr})")
                } else if is_opt {
                    format!("fmt.Sprint(*{field_expr})")
                } else if field_is_array {
                    format!("jsonString({field_expr})")
                } else {
                    format!("fmt.Sprint({field_expr})")
                };
                let _ = writeln!(out_ref, "\t{{");
                let _ = writeln!(out_ref, "\t\tfound := false");
                for val in values {
                    let go_val = json_to_go(val);
                    let _ = writeln!(
                        out_ref,
                        "\t\tif strings.Contains({field_for_contains}, {go_val}) {{ found = true }}"
                    );
                }
                let _ = writeln!(out_ref, "\t\tif !found {{");
                let _ = writeln!(
                    out_ref,
                    "\t\t\tt.Errorf(\"expected to contain at least one of the specified values\")"
                );
                let _ = writeln!(out_ref, "\t\t}}");
                let _ = writeln!(out_ref, "\t}}");
            }
        }
        "greater_than" => {
            if let Some(val) = &assertion.value {
                let go_val = json_to_go(val);
                // Use `< N+1` instead of `<= N` to avoid golangci-lint sloppyLen
                // warning when N is 0 (len(x) <= 0 → len(x) < 1).
                // For optional (pointer) fields, dereference and guard with nil check.
                if is_optional {
                    let _ = writeln!(out_ref, "\tif {field_expr} != nil {{");
                    if let Some(n) = val.as_u64() {
                        let next = n + 1;
                        let _ = writeln!(out_ref, "\t\tif {deref_field_expr} < {next} {{");
                    } else {
                        let _ = writeln!(out_ref, "\t\tif {deref_field_expr} <= {go_val} {{");
                    }
                    let _ = writeln!(
                        out_ref,
                        "\t\t\tt.Errorf(\"expected > {go_val}, got %v\", {deref_field_expr})"
                    );
                    let _ = writeln!(out_ref, "\t\t}}");
                    let _ = writeln!(out_ref, "\t}}");
                } else if let Some(n) = val.as_u64() {
                    let next = n + 1;
                    let _ = writeln!(out_ref, "\tif {field_expr} < {next} {{");
                    let _ = writeln!(out_ref, "\t\tt.Errorf(\"expected > {go_val}, got %v\", {field_expr})");
                    let _ = writeln!(out_ref, "\t}}");
                } else {
                    let _ = writeln!(out_ref, "\tif {field_expr} <= {go_val} {{");
                    let _ = writeln!(out_ref, "\t\tt.Errorf(\"expected > {go_val}, got %v\", {field_expr})");
                    let _ = writeln!(out_ref, "\t}}");
                }
            }
        }
        "less_than" => {
            if let Some(val) = &assertion.value {
                let go_val = json_to_go(val);
                let _ = writeln!(out_ref, "\tif {field_expr} >= {go_val} {{");
                let _ = writeln!(out_ref, "\t\tt.Errorf(\"expected < {go_val}, got %v\", {field_expr})");
                let _ = writeln!(out_ref, "\t}}");
            }
        }
        "greater_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let go_val = json_to_go(val);
                if let Some(ref guard) = nil_guard_expr {
                    let _ = writeln!(out_ref, "\tif {guard} != nil {{");
                    let _ = writeln!(out_ref, "\t\tif {field_expr} < {go_val} {{");
                    let _ = writeln!(
                        out_ref,
                        "\t\t\tt.Errorf(\"expected >= {go_val}, got %v\", {field_expr})"
                    );
                    let _ = writeln!(out_ref, "\t\t}}");
                    let _ = writeln!(out_ref, "\t}}");
                } else if is_optional && !field_expr.starts_with("len(") {
                    // Optional pointer field: nil-guard and dereference before comparison.
                    let _ = writeln!(out_ref, "\tif {field_expr} != nil {{");
                    let _ = writeln!(out_ref, "\t\tif {deref_field_expr} < {go_val} {{");
                    let _ = writeln!(
                        out_ref,
                        "\t\t\tt.Errorf(\"expected >= {go_val}, got %v\", {deref_field_expr})"
                    );
                    let _ = writeln!(out_ref, "\t\t}}");
                    let _ = writeln!(out_ref, "\t}}");
                } else {
                    let _ = writeln!(out_ref, "\tif {field_expr} < {go_val} {{");
                    let _ = writeln!(out_ref, "\t\tt.Errorf(\"expected >= {go_val}, got %v\", {field_expr})");
                    let _ = writeln!(out_ref, "\t}}");
                }
            }
        }
        "less_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let go_val = json_to_go(val);
                let _ = writeln!(out_ref, "\tif {field_expr} > {go_val} {{");
                let _ = writeln!(out_ref, "\t\tt.Errorf(\"expected <= {go_val}, got %v\", {field_expr})");
                let _ = writeln!(out_ref, "\t}}");
            }
        }
        "starts_with" => {
            if let Some(expected) = &assertion.value {
                let go_val = json_to_go(expected);
                let field_for_prefix = if is_optional
                    && !optional_locals.contains_key(assertion.field.as_ref().unwrap_or(&String::new()))
                {
                    format!("string(*{field_expr})")
                } else {
                    format!("string({field_expr})")
                };
                let _ = writeln!(out_ref, "\tif !strings.HasPrefix({field_for_prefix}, {go_val}) {{");
                let _ = writeln!(
                    out_ref,
                    "\t\tt.Errorf(\"expected to start with %s, got %v\", {go_val}, {field_expr})"
                );
                let _ = writeln!(out_ref, "\t}}");
            }
        }
        "count_min" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    if is_optional {
                        let _ = writeln!(out_ref, "\tif {field_expr} != nil {{");
                        let _ = writeln!(
                            out_ref,
                            "\t\tassert.GreaterOrEqual(t, len(*{field_expr}), {n}, \"expected at least {n} elements\")"
                        );
                        let _ = writeln!(out_ref, "\t}}");
                    } else {
                        let _ = writeln!(
                            out_ref,
                            "\tassert.GreaterOrEqual(t, len({field_expr}), {n}, \"expected at least {n} elements\")"
                        );
                    }
                }
            }
        }
        "count_equals" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    if is_optional {
                        let _ = writeln!(out_ref, "\tif {field_expr} != nil {{");
                        let _ = writeln!(
                            out_ref,
                            "\t\tassert.Equal(t, len(*{field_expr}), {n}, \"expected exactly {n} elements\")"
                        );
                        let _ = writeln!(out_ref, "\t}}");
                    } else {
                        let _ = writeln!(
                            out_ref,
                            "\tassert.Equal(t, len({field_expr}), {n}, \"expected exactly {n} elements\")"
                        );
                    }
                }
            }
        }
        "is_true" => {
            if is_optional {
                let _ = writeln!(out_ref, "\tif {field_expr} != nil {{");
                let _ = writeln!(out_ref, "\t\tassert.True(t, *{field_expr}, \"expected true\")");
                let _ = writeln!(out_ref, "\t}}");
            } else {
                let _ = writeln!(out_ref, "\tassert.True(t, {field_expr}, \"expected true\")");
            }
        }
        "is_false" => {
            if is_optional {
                let _ = writeln!(out_ref, "\tif {field_expr} != nil {{");
                let _ = writeln!(out_ref, "\t\tassert.False(t, *{field_expr}, \"expected false\")");
                let _ = writeln!(out_ref, "\t}}");
            } else {
                let _ = writeln!(out_ref, "\tassert.False(t, {field_expr}, \"expected false\")");
            }
        }
        "method_result" => {
            if let Some(method_name) = &assertion.method {
                let info = build_go_method_call(result_var, method_name, assertion.args.as_ref(), import_alias);
                let check = assertion.check.as_deref().unwrap_or("is_true");
                // For pointer-returning functions, dereference with `*`. Value-returning
                // functions (e.g., NodeInfo field access) are used directly.
                let deref_expr = if info.is_pointer {
                    format!("*{}", info.call_expr)
                } else {
                    info.call_expr.clone()
                };
                match check {
                    "equals" => {
                        if let Some(val) = &assertion.value {
                            if val.is_boolean() {
                                if val.as_bool() == Some(true) {
                                    let _ = writeln!(out_ref, "\tassert.True(t, {deref_expr}, \"expected true\")");
                                } else {
                                    let _ = writeln!(out_ref, "\tassert.False(t, {deref_expr}, \"expected false\")");
                                }
                            } else {
                                // Apply type cast to numeric literals when the method returns
                                // a typed uint (e.g., *uint) to avoid reflect.DeepEqual
                                // mismatches between int and uint in testify's assert.Equal.
                                let go_val = if let Some(cast) = info.value_cast {
                                    if val.is_number() {
                                        format!("{cast}({})", json_to_go(val))
                                    } else {
                                        json_to_go(val)
                                    }
                                } else {
                                    json_to_go(val)
                                };
                                let _ = writeln!(
                                    out_ref,
                                    "\tassert.Equal(t, {go_val}, {deref_expr}, \"method_result equals assertion failed\")"
                                );
                            }
                        }
                    }
                    "is_true" => {
                        let _ = writeln!(out_ref, "\tassert.True(t, {deref_expr}, \"expected true\")");
                    }
                    "is_false" => {
                        let _ = writeln!(out_ref, "\tassert.False(t, {deref_expr}, \"expected false\")");
                    }
                    "greater_than_or_equal" => {
                        if let Some(val) = &assertion.value {
                            let n = val.as_u64().unwrap_or(0);
                            // Use the value_cast type if available (e.g., uint for named_children_count).
                            let cast = info.value_cast.unwrap_or("uint");
                            let _ = writeln!(
                                out_ref,
                                "\tassert.GreaterOrEqual(t, {deref_expr}, {cast}({n}), \"expected >= {n}\")"
                            );
                        }
                    }
                    "count_min" => {
                        if let Some(val) = &assertion.value {
                            let n = val.as_u64().unwrap_or(0);
                            let _ = writeln!(
                                out_ref,
                                "\tassert.GreaterOrEqual(t, len({deref_expr}), {n}, \"expected at least {n} elements\")"
                            );
                        }
                    }
                    "contains" => {
                        if let Some(val) = &assertion.value {
                            let go_val = json_to_go(val);
                            let _ = writeln!(
                                out_ref,
                                "\tassert.Contains(t, {deref_expr}, {go_val}, \"expected result to contain value\")"
                            );
                        }
                    }
                    "is_error" => {
                        let _ = writeln!(out_ref, "\t{{");
                        let _ = writeln!(out_ref, "\t\t_, methodErr := {}", info.call_expr);
                        let _ = writeln!(out_ref, "\t\tassert.Error(t, methodErr)");
                        let _ = writeln!(out_ref, "\t}}");
                    }
                    other_check => {
                        panic!("Go e2e generator: unsupported method_result check type: {other_check}");
                    }
                }
            } else {
                panic!("Go e2e generator: method_result assertion missing 'method' field");
            }
        }
        "min_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    if is_optional {
                        let _ = writeln!(out_ref, "\tif {field_expr} != nil {{");
                        let _ = writeln!(
                            out_ref,
                            "\t\tassert.GreaterOrEqual(t, len(*{field_expr}), {n}, \"expected length >= {n}\")"
                        );
                        let _ = writeln!(out_ref, "\t}}");
                    } else if field_expr.starts_with("len(") {
                        let _ = writeln!(
                            out_ref,
                            "\tassert.GreaterOrEqual(t, {field_expr}, {n}, \"expected length >= {n}\")"
                        );
                    } else {
                        let _ = writeln!(
                            out_ref,
                            "\tassert.GreaterOrEqual(t, len({field_expr}), {n}, \"expected length >= {n}\")"
                        );
                    }
                }
            }
        }
        "max_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    if is_optional {
                        let _ = writeln!(out_ref, "\tif {field_expr} != nil {{");
                        let _ = writeln!(
                            out_ref,
                            "\t\tassert.LessOrEqual(t, len(*{field_expr}), {n}, \"expected length <= {n}\")"
                        );
                        let _ = writeln!(out_ref, "\t}}");
                    } else if field_expr.starts_with("len(") {
                        let _ = writeln!(
                            out_ref,
                            "\tassert.LessOrEqual(t, {field_expr}, {n}, \"expected length <= {n}\")"
                        );
                    } else {
                        let _ = writeln!(
                            out_ref,
                            "\tassert.LessOrEqual(t, len({field_expr}), {n}, \"expected length <= {n}\")"
                        );
                    }
                }
            }
        }
        "ends_with" => {
            if let Some(expected) = &assertion.value {
                let go_val = json_to_go(expected);
                let field_for_suffix = if is_optional
                    && !optional_locals.contains_key(assertion.field.as_ref().unwrap_or(&String::new()))
                {
                    format!("string(*{field_expr})")
                } else {
                    format!("string({field_expr})")
                };
                let _ = writeln!(out_ref, "\tif !strings.HasSuffix({field_for_suffix}, {go_val}) {{");
                let _ = writeln!(
                    out_ref,
                    "\t\tt.Errorf(\"expected to end with %s, got %v\", {go_val}, {field_expr})"
                );
                let _ = writeln!(out_ref, "\t}}");
            }
        }
        "matches_regex" => {
            if let Some(expected) = &assertion.value {
                let go_val = json_to_go(expected);
                let field_for_regex = if is_optional
                    && !optional_locals.contains_key(assertion.field.as_ref().unwrap_or(&String::new()))
                {
                    format!("*{field_expr}")
                } else {
                    field_expr.clone()
                };
                let _ = writeln!(
                    out_ref,
                    "\tassert.Regexp(t, {go_val}, {field_for_regex}, \"expected value to match regex\")"
                );
            }
        }
        "not_error" => {
            // Already handled by the `if err != nil` check above.
        }
        "error" => {
            // Handled at the test function level.
        }
        other => {
            panic!("Go e2e generator: unsupported assertion type: {other}");
        }
    }

    // If the assertion accesses an array element via [0], wrap the generated code in a
    // bounds check to prevent an index-out-of-range panic when the array is empty.
    if let Some(ref arr) = array_guard {
        if !assertion_buf.is_empty() {
            let _ = writeln!(out, "\tif len({arr}) > 0 {{");
            // Re-indent each line by one additional tab level.
            for line in assertion_buf.lines() {
                let _ = writeln!(out, "\t{line}");
            }
            let _ = writeln!(out, "\t}}");
        }
    } else {
        out.push_str(&assertion_buf);
    }
}

/// Metadata about the return type of a Go method call for `method_result` assertions.
struct GoMethodCallInfo {
    /// The call expression string.
    call_expr: String,
    /// Whether the return type is a pointer (needs `*` dereference for value comparison).
    is_pointer: bool,
    /// Optional Go type cast to apply to numeric literal values in `equals` assertions
    /// (e.g., `"uint"` so that `0` becomes `uint(0)` to match `*uint` deref type).
    value_cast: Option<&'static str>,
}

/// Build a Go call expression for a `method_result` assertion on a tree-sitter Tree.
///
/// Maps method names to the appropriate Go function calls, matching the Go binding API
/// in `packages/go/binding.go`. Returns a [`GoMethodCallInfo`] describing the call and
/// its return type characteristics.
///
/// Return types by method:
/// - `has_error_nodes`, `contains_node_type` → `*bool` (pointer)
/// - `error_count` → `*uint` (pointer, value_cast = "uint")
/// - `tree_to_sexp` → `*string` (pointer)
/// - `root_node_type` → `string` via `RootNodeInfo(tree).Kind` (value)
/// - `named_children_count` → `uint` via `RootNodeInfo(tree).NamedChildCount` (value, value_cast = "uint")
/// - `find_nodes_by_type` → `*[]NodeInfo` (pointer to slice)
/// - `run_query` → `(*[]QueryMatch, error)` (pointer + error; use `is_error` check type)
fn build_go_method_call(
    result_var: &str,
    method_name: &str,
    args: Option<&serde_json::Value>,
    import_alias: &str,
) -> GoMethodCallInfo {
    match method_name {
        "root_node_type" => GoMethodCallInfo {
            call_expr: format!("{import_alias}.RootNodeInfo({result_var}).Kind"),
            is_pointer: false,
            value_cast: None,
        },
        "named_children_count" => GoMethodCallInfo {
            call_expr: format!("{import_alias}.RootNodeInfo({result_var}).NamedChildCount"),
            is_pointer: false,
            value_cast: Some("uint"),
        },
        "has_error_nodes" => GoMethodCallInfo {
            call_expr: format!("{import_alias}.TreeHasErrorNodes({result_var})"),
            is_pointer: true,
            value_cast: None,
        },
        "error_count" | "tree_error_count" => GoMethodCallInfo {
            call_expr: format!("{import_alias}.TreeErrorCount({result_var})"),
            is_pointer: true,
            value_cast: Some("uint"),
        },
        "tree_to_sexp" => GoMethodCallInfo {
            call_expr: format!("{import_alias}.TreeToSexp({result_var})"),
            is_pointer: true,
            value_cast: None,
        },
        "contains_node_type" => {
            let node_type = args
                .and_then(|a| a.get("node_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            GoMethodCallInfo {
                call_expr: format!("{import_alias}.TreeContainsNodeType({result_var}, \"{node_type}\")"),
                is_pointer: true,
                value_cast: None,
            }
        }
        "find_nodes_by_type" => {
            let node_type = args
                .and_then(|a| a.get("node_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            GoMethodCallInfo {
                call_expr: format!("{import_alias}.FindNodesByType({result_var}, \"{node_type}\")"),
                is_pointer: true,
                value_cast: None,
            }
        }
        "run_query" => {
            let query_source = args
                .and_then(|a| a.get("query_source"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let language = args
                .and_then(|a| a.get("language"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let query_lit = go_string_literal(query_source);
            let lang_lit = go_string_literal(language);
            // RunQuery returns (*[]QueryMatch, error) — use is_error check type.
            GoMethodCallInfo {
                call_expr: format!("{import_alias}.RunQuery({result_var}, {lang_lit}, {query_lit}, []byte(source))"),
                is_pointer: false,
                value_cast: None,
            }
        }
        other => {
            let method_pascal = other.to_upper_camel_case();
            GoMethodCallInfo {
                call_expr: format!("{result_var}.{method_pascal}()"),
                is_pointer: false,
                value_cast: None,
            }
        }
    }
}

/// Convert a `serde_json::Value` to a Go literal string.
/// Recursively convert a JSON value for Go struct unmarshalling.
///
/// The Go binding's `ConversionOptions` struct uses:
/// - `snake_case` JSON field tags (e.g. `"code_block_style"` not `"codeBlockStyle"`)
/// - lowercase/snake_case string values for enums (e.g. `"indented"`, `"atx_closed"`)
///
/// Fixture JSON uses camelCase keys and PascalCase enum values (Python/TS conventions).
/// This function remaps both so the generated Go tests can unmarshal correctly.
fn convert_json_for_go(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let new_map: serde_json::Map<String, serde_json::Value> = map
                .into_iter()
                .map(|(k, v)| (camel_to_snake_case(&k), convert_json_for_go(v)))
                .collect();
            serde_json::Value::Object(new_map)
        }
        serde_json::Value::Array(arr) => serde_json::Value::Array(arr.into_iter().map(convert_json_for_go).collect()),
        serde_json::Value::String(s) => {
            // Convert PascalCase enum values to snake_case.
            // Only convert values that look like PascalCase (start with uppercase, no spaces).
            serde_json::Value::String(pascal_to_snake_case(&s))
        }
        other => other,
    }
}

/// Convert a camelCase or PascalCase string to snake_case.
fn camel_to_snake_case(s: &str) -> String {
    let mut result = String::new();
    let mut prev_upper = false;
    for (i, c) in s.char_indices() {
        if c.is_uppercase() {
            if i > 0 && !prev_upper {
                result.push('_');
            }
            result.push(c.to_lowercase().next().unwrap_or(c));
            prev_upper = true;
        } else {
            if prev_upper && i > 1 {
                // Handles sequences like "URLPath" → "url_path": insert _ before last uppercase
                // when transitioning from a run of uppercase back to lowercase.
                // This is tricky — use simple approach: detect Aa pattern.
            }
            result.push(c);
            prev_upper = false;
        }
    }
    result
}

/// Convert a PascalCase string to snake_case (for enum values).
///
/// Only converts if the string looks like PascalCase (starts uppercase, no spaces/underscores).
/// Values that are already lowercase/snake_case are returned unchanged.
fn pascal_to_snake_case(s: &str) -> String {
    // Skip conversion for strings that already contain underscores, spaces, or start lowercase.
    let first_char = s.chars().next();
    if first_char.is_none() || !first_char.unwrap().is_uppercase() || s.contains('_') || s.contains(' ') {
        return s.to_string();
    }
    camel_to_snake_case(s)
}

/// Map an `ArgMapping.element_type` to a Go slice type. Used for `json_object` args
/// whose fixture value is a JSON array. The element type is wrapped in `[]…` so an
/// element of `String` becomes `[]string` and `Vec<String>` becomes `[][]string`.
fn element_type_to_go_slice(element_type: Option<&str>, import_alias: &str) -> String {
    let elem = element_type.unwrap_or("String").trim();
    let go_elem = rust_type_to_go(elem, import_alias);
    format!("[]{go_elem}")
}

/// Map a small subset of Rust scalar / `Vec<T>` types to their Go equivalents.
/// For unknown types, qualify with the import alias (e.g., "kreuzberg.BatchBytesItem").
fn rust_type_to_go(rust: &str, import_alias: &str) -> String {
    let trimmed = rust.trim();
    if let Some(inner) = trimmed.strip_prefix("Vec<").and_then(|s| s.strip_suffix('>')) {
        return format!("[]{}", rust_type_to_go(inner, import_alias));
    }
    match trimmed {
        "String" | "&str" | "str" => "string".to_string(),
        "bool" => "bool".to_string(),
        "f32" => "float32".to_string(),
        "f64" => "float64".to_string(),
        "i8" => "int8".to_string(),
        "i16" => "int16".to_string(),
        "i32" => "int32".to_string(),
        "i64" | "isize" => "int64".to_string(),
        "u8" => "uint8".to_string(),
        "u16" => "uint16".to_string(),
        "u32" => "uint32".to_string(),
        "u64" | "usize" => "uint64".to_string(),
        _ => format!("{import_alias}.{trimmed}"),
    }
}

fn json_to_go(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => go_string_literal(s),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Null => "nil".to_string(),
        // For complex types, serialize to JSON string and pass as literal.
        other => go_string_literal(&other.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Visitor generation
// ---------------------------------------------------------------------------

/// Derive a unique, exported Go struct name for a visitor from a fixture ID.
///
/// E.g. `visitor_continue_default` → `visitorContinueDefault` (unexported, avoids
/// polluting the exported API of the test package while still being package-level).
fn visitor_struct_name(fixture_id: &str) -> String {
    use heck::ToUpperCamelCase;
    // Use UpperCamelCase so Go treats it as exported — required for method sets.
    format!("testVisitor{}", fixture_id.to_upper_camel_case())
}

/// Emit a package-level Go struct declaration and all its visitor methods.
///
/// The struct embeds `BaseVisitor` to satisfy all interface methods not
/// explicitly overridden by the fixture callbacks.
fn emit_go_visitor_struct(
    out: &mut String,
    struct_name: &str,
    visitor_spec: &crate::fixture::VisitorSpec,
    import_alias: &str,
) {
    let _ = writeln!(out, "type {struct_name} struct{{");
    let _ = writeln!(out, "\t{import_alias}.BaseVisitor");
    let _ = writeln!(out, "}}");
    for (method_name, action) in &visitor_spec.callbacks {
        emit_go_visitor_method(out, struct_name, method_name, action, import_alias);
    }
}

/// Emit a Go visitor method for a callback action on the named struct.
fn emit_go_visitor_method(
    out: &mut String,
    struct_name: &str,
    method_name: &str,
    action: &CallbackAction,
    import_alias: &str,
) {
    let camel_method = method_to_camel(method_name);
    // Parameter signatures must exactly match the htmltomarkdown.Visitor interface.
    // Optional fields use pointer types (*string, *uint32, etc.) to indicate nil-ability.
    let params = match method_name {
        "visit_link" => format!("_ {import_alias}.NodeContext, href string, text string, title *string"),
        "visit_image" => format!("_ {import_alias}.NodeContext, src string, alt string, title *string"),
        "visit_heading" => format!("_ {import_alias}.NodeContext, level uint32, text string, id *string"),
        "visit_code_block" => format!("_ {import_alias}.NodeContext, lang *string, code string"),
        "visit_code_inline"
        | "visit_strong"
        | "visit_emphasis"
        | "visit_strikethrough"
        | "visit_underline"
        | "visit_subscript"
        | "visit_superscript"
        | "visit_mark"
        | "visit_button"
        | "visit_summary"
        | "visit_figcaption"
        | "visit_definition_term"
        | "visit_definition_description" => format!("_ {import_alias}.NodeContext, text string"),
        "visit_text" => format!("_ {import_alias}.NodeContext, text string"),
        "visit_list_item" => {
            format!("_ {import_alias}.NodeContext, ordered bool, marker string, text string")
        }
        "visit_blockquote" => format!("_ {import_alias}.NodeContext, content string, depth uint"),
        "visit_table_row" => format!("_ {import_alias}.NodeContext, cells []string, isHeader bool"),
        "visit_custom_element" => format!("_ {import_alias}.NodeContext, tagName string, html string"),
        "visit_form" => format!("_ {import_alias}.NodeContext, action *string, method *string"),
        "visit_input" => {
            format!("_ {import_alias}.NodeContext, inputType string, name *string, value *string")
        }
        "visit_audio" | "visit_video" | "visit_iframe" => {
            format!("_ {import_alias}.NodeContext, src *string")
        }
        "visit_details" => format!("_ {import_alias}.NodeContext, open bool"),
        "visit_element_end" | "visit_table_end" | "visit_definition_list_end" | "visit_figure_end" => {
            format!("_ {import_alias}.NodeContext, output string")
        }
        "visit_list_start" => format!("_ {import_alias}.NodeContext, ordered bool"),
        "visit_list_end" => format!("_ {import_alias}.NodeContext, ordered bool, output string"),
        _ => format!("_ {import_alias}.NodeContext"),
    };

    let _ = writeln!(
        out,
        "func (v *{struct_name}) {camel_method}({params}) {import_alias}.VisitResult {{"
    );
    match action {
        CallbackAction::Skip => {
            let _ = writeln!(out, "\treturn {import_alias}.VisitResultSkip()");
        }
        CallbackAction::Continue => {
            let _ = writeln!(out, "\treturn {import_alias}.VisitResultContinue()");
        }
        CallbackAction::PreserveHtml => {
            let _ = writeln!(out, "\treturn {import_alias}.VisitResultPreserveHTML()");
        }
        CallbackAction::Custom { output } => {
            let escaped = go_string_literal(output);
            let _ = writeln!(out, "\treturn {import_alias}.VisitResultCustom({escaped})");
        }
        CallbackAction::CustomTemplate { template } => {
            // Convert {var} placeholders to %s format verbs and collect arg names.
            // E.g. `QUOTE: "{text}"` → fmt.Sprintf("QUOTE: \"%s\"", text)
            //
            // For pointer-typed params (e.g. `src *string`), dereference with `*`
            // — the test fixtures always supply a non-nil value for methods that
            // fire a custom template, so this is safe in practice.
            let ptr_params = go_visitor_ptr_params(method_name);
            let (fmt_str, fmt_args) = template_to_sprintf(template, &ptr_params);
            let escaped_fmt = go_string_literal(&fmt_str);
            if fmt_args.is_empty() {
                let _ = writeln!(out, "\treturn {import_alias}.VisitResultCustom({escaped_fmt})");
            } else {
                let args_str = fmt_args.join(", ");
                let _ = writeln!(
                    out,
                    "\treturn {import_alias}.VisitResultCustom(fmt.Sprintf({escaped_fmt}, {args_str}))"
                );
            }
        }
    }
    let _ = writeln!(out, "}}");
}

/// Return the set of camelCase parameter names that are pointer types (`*string`) for a
/// given visitor method name.  Used to dereference pointers in template `fmt.Sprintf` calls.
fn go_visitor_ptr_params(method_name: &str) -> std::collections::HashSet<&'static str> {
    match method_name {
        "visit_link" => ["title"].into(),
        "visit_image" => ["title"].into(),
        "visit_heading" => ["id"].into(),
        "visit_code_block" => ["lang"].into(),
        "visit_form" => ["action", "method"].into(),
        "visit_input" => ["name", "value"].into(),
        "visit_audio" | "visit_video" | "visit_iframe" => ["src"].into(),
        _ => std::collections::HashSet::new(),
    }
}

/// Convert a `{var}` template string into a `fmt.Sprintf` format string and argument list.
///
/// For example, `QUOTE: "{text}"` becomes `("QUOTE: \"%s\"", vec!["text"])`.
///
/// Placeholder names in the template use snake_case (matching fixture field names); they
/// are converted to Go camelCase parameter names using `go_param_name` so they match the
/// generated visitor method signatures (e.g. `{input_type}` → `inputType`).
///
/// `ptr_params` — camelCase names of parameters that are `*string`; these are
/// dereferenced with `*` when used as `fmt.Sprintf` arguments.  The fixtures that
/// use `custom_template` on pointer-param methods always supply a non-nil value.
fn template_to_sprintf(template: &str, ptr_params: &std::collections::HashSet<&str>) -> (String, Vec<String>) {
    let mut fmt_str = String::new();
    let mut args: Vec<String> = Vec::new();
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '{' {
            // Collect placeholder name until '}'.
            let mut name = String::new();
            for inner in chars.by_ref() {
                if inner == '}' {
                    break;
                }
                name.push(inner);
            }
            fmt_str.push_str("%s");
            // Convert snake_case placeholder to Go camelCase to match method param names.
            let go_name = go_param_name(&name);
            // Dereference pointer params so fmt.Sprintf receives a string value.
            let arg_expr = if ptr_params.contains(go_name.as_str()) {
                format!("*{go_name}")
            } else {
                go_name
            };
            args.push(arg_expr);
        } else {
            fmt_str.push(c);
        }
    }
    (fmt_str, args)
}

/// Convert snake_case method names to Go camelCase.
fn method_to_camel(snake: &str) -> String {
    use heck::ToUpperCamelCase;
    snake.to_upper_camel_case()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{CallConfig, E2eConfig};
    use crate::field_access::FieldResolver;
    use crate::fixture::{Assertion, Fixture};

    fn make_fixture(id: &str) -> Fixture {
        Fixture {
            id: id.to_string(),
            category: None,
            description: "test fixture".to_string(),
            tags: vec![],
            skip: None,
            call: None,
            input: serde_json::Value::Null,
            mock_response: Some(crate::fixture::MockResponse {
                status: 200,
                body: Some(serde_json::Value::Null),
                stream_chunks: None,
                headers: std::collections::HashMap::new(),
            }),
            source: String::new(),
            http: None,
            assertions: vec![Assertion {
                assertion_type: "not_error".to_string(),
                field: None,
                value: None,
                values: None,
                method: None,
                args: None,
                check: None,
            }],
            visitor: None,
        }
    }

    /// snake_case function names in `[e2e.call]` must be routed through `to_go_name`
    /// so the emitted Go call uses the idiomatic CamelCase (e.g. `CleanExtractedText`
    /// instead of `clean_extracted_text`).
    #[test]
    fn test_go_method_name_uses_go_casing() {
        let e2e_config = E2eConfig {
            call: CallConfig {
                function: "clean_extracted_text".to_string(),
                module: "github.com/example/mylib".to_string(),
                result_var: "result".to_string(),
                returns_result: true,
                ..CallConfig::default()
            },
            ..E2eConfig::default()
        };

        let fixture = make_fixture("basic_text");
        let resolver = FieldResolver::new(
            &std::collections::HashMap::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
        );
        let mut out = String::new();
        render_test_function(&mut out, &fixture, "kreuzberg", &resolver, &e2e_config);

        assert!(
            out.contains("kreuzberg.CleanExtractedText("),
            "expected Go-cased method name 'CleanExtractedText', got:\n{out}"
        );
        assert!(
            !out.contains("kreuzberg.clean_extracted_text("),
            "must not emit raw snake_case method name, got:\n{out}"
        );
    }
}
