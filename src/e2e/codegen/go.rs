//! Go e2e test generator using testing.T.

use crate::core::backend::GeneratedFile;
use crate::core::config::Language;
use crate::core::config::ResolvedCrateConfig;
use crate::core::hash::{self, CommentStyle};
use crate::e2e::config::E2eConfig;
use crate::e2e::escape::sanitize_filename;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::{Fixture, FixtureGroup};
use anyhow::Result;
use heck::ToUpperCamelCase;
use std::fmt::Write as FmtWrite;
use std::path::PathBuf;

use super::E2eCodegen;
use super::client;

fn resolve_handle_config_type(
    arg: &crate::e2e::config::ArgMapping,
    options_type: Option<&str>,
    type_defs: &[crate::core::ir::TypeDef],
) -> Option<String> {
    if arg.arg_type != "handle" {
        return None;
    }
    options_type.map(str::to_string).or_else(|| {
        let candidate = format!("{}Config", arg.name.to_upper_camel_case());
        type_defs.iter().any(|ty| ty.name == candidate).then_some(candidate)
    })
}

/// Go e2e code generator.
pub struct GoCodegen;

impl E2eCodegen for GoCodegen {
    fn generate(
        &self,
        groups: &[FixtureGroup],
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        type_defs: &[crate::core::ir::TypeDef],
        enums: &[crate::core::ir::EnumDef],
    ) -> Result<Vec<GeneratedFile>> {
        let lang = self.language_name();
        let output_base = PathBuf::from(e2e_config.effective_output()).join(lang);

        let mut files = Vec::new();

        // Identify data-enum (sum-type) names: enums where at least one variant has named fields.
        // These require special unmarshaling in Go (via Unmarshal<Type> discriminator).
        let data_enum_names: std::collections::HashSet<&str> = enums
            .iter()
            .filter(|e| {
                e.variants
                    .iter()
                    .any(|v| !v.fields.is_empty() && v.fields.iter().any(|f| !f.name.is_empty()))
            })
            .map(|e| e.name.as_str())
            .collect();

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
        // In Local mode the version field is a placeholder for the
        // `require <module> <version>` line that pairs with a `replace`
        // directive — `v0.0.0` is the common idiom and gets corrected by
        // `fix_go_major_version` if the module is `/vN` for N >= 2.
        //
        // In Registry mode the test app is consuming the published module
        // from the Go proxy, so the version must match the real release tag.
        // The merged `PackageRef.version` falls through to the Local-mode
        // `[crates.e2e.packages.go].version` placeholder when no
        // `[crates.e2e.registry.packages.go].version` is configured, which
        // wrote `v0.0.0` into the published test_app's go.mod. Prefer the
        // crate-wide resolved version (from `version_from`) in Registry mode
        // and fall back to the package's version only if no resolved version
        // is available.
        let go_version = match e2e_config.dep_mode {
            crate::e2e::config::DependencyMode::Registry => {
                let registry_version = e2e_config
                    .registry
                    .packages
                    .get("go")
                    .and_then(|p| p.version.as_ref())
                    .cloned();
                registry_version
                    .or_else(|| config.resolved_version().map(|v| format!("v{v}")))
                    .or_else(|| go_pkg.as_ref().and_then(|p| p.version.as_ref()).cloned())
                    .unwrap_or_else(|| "v0.0.0".to_string())
            }
            crate::e2e::config::DependencyMode::Local => go_pkg
                .as_ref()
                .and_then(|p| p.version.as_ref())
                .cloned()
                .unwrap_or_else(|| {
                    config
                        .resolved_version()
                        .map(|v| format!("v{v}"))
                        .unwrap_or_else(|| "v0.0.0".to_string())
                }),
        };
        // Generate go.mod. In registry mode, omit the `replace` directive so the
        // module is fetched from the Go module proxy.
        let effective_replace = match e2e_config.dep_mode {
            crate::e2e::config::DependencyMode::Registry => None,
            crate::e2e::config::DependencyMode::Local => replace_path.as_deref().map(String::from),
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

        // Determine if any fixture needs jsonString helper across all groups.
        let emits_executable_test =
            |fixture: &Fixture| fixture.is_http_test() || fixture_has_go_callable(fixture, e2e_config);
        let needs_json_stringify = groups.iter().flat_map(|g| g.fixtures.iter()).any(|f| {
            emits_executable_test(f)
                && f.assertions.iter().any(|a| {
                    matches!(
                        a.assertion_type.as_str(),
                        "contains" | "contains_all" | "contains_any" | "not_contains"
                    ) && {
                        if a.field.as_ref().is_none_or(|f| f.is_empty()) {
                            e2e_config
                                .resolve_call_for_fixture(
                                    f.call.as_deref(),
                                    &f.id,
                                    &f.resolved_category(),
                                    &f.tags,
                                    &f.input,
                                )
                                .result_is_array
                        } else {
                            let cc = e2e_config.resolve_call_for_fixture(
                                f.call.as_deref(),
                                &f.id,
                                &f.resolved_category(),
                                &f.tags,
                                &f.input,
                            );
                            let per_call_resolver = FieldResolver::new(
                                e2e_config.effective_fields(cc),
                                e2e_config.effective_fields_optional(cc),
                                e2e_config.effective_result_fields(cc),
                                e2e_config.effective_fields_array(cc),
                                &std::collections::HashSet::new(),
                            );
                            let resolved_name = per_call_resolver.resolve(a.field.as_deref().unwrap_or(""));
                            per_call_resolver.is_array(resolved_name)
                        }
                    }
                })
        });

        // Generate helpers_test.go with jsonString if needed, emitted exactly once.
        if needs_json_stringify {
            files.push(GeneratedFile {
                path: output_base.join("helpers_test.go"),
                content: render_helpers_test_go(),
                generated_header: true,
            });
        }

        // Generate main_test.go with TestMain when:
        // 1. Any fixture needs the mock server (has mock_response), or
        // 2. Any fixture is client_factory-based (reads MOCK_SERVER_URL), or
        // 3. Any fixture is file-based (requires test_documents directory setup).
        //
        // TestMain runs before all tests and changes to the test_documents directory,
        // ensuring that relative file paths like "pdf/fake_memo.pdf" resolve correctly.
        let has_file_fixtures = groups
            .iter()
            .flat_map(|g| g.fixtures.iter())
            .any(|f| f.http.is_none() && !f.needs_mock_server());

        // Determine if any fixture needs the mock-server binary or HTTP integration tests.
        let needs_mock_server = groups
            .iter()
            .flat_map(|g| g.fixtures.iter())
            .any(|f| f.http.is_none() && f.needs_mock_server());
        let needs_http_tests = groups.iter().flat_map(|g| g.fixtures.iter()).any(|f| f.http.is_some());

        let needs_main_test = has_file_fixtures
            || needs_http_tests
            || groups.iter().flat_map(|g| g.fixtures.iter()).any(|f| {
                if f.http.is_none() && f.needs_mock_server() {
                    return true;
                }
                let cc = e2e_config.resolve_call_for_fixture(
                    f.call.as_deref(),
                    &f.id,
                    &f.resolved_category(),
                    &f.tags,
                    &f.input,
                );
                let go_override = cc.overrides.get("go").or_else(|| e2e_config.call.overrides.get("go"));
                go_override.and_then(|o| o.client_factory.as_deref()).is_some()
            });

        // Emit cmd/harness/main.go when HTTP fixtures are present.
        let has_http_fixtures = needs_http_tests;

        if needs_main_test {
            files.push(GeneratedFile {
                path: output_base.join("main_test.go"),
                content: render_main_test_go(&e2e_config.test_documents_dir, needs_mock_server, has_http_fixtures),
                generated_header: true,
            });

            if has_http_fixtures {
                let harness_content = render_harness_main(e2e_config, groups, &module_path);
                files.push(GeneratedFile {
                    path: output_base.join("cmd").join("harness").join("main.go"),
                    content: harness_content,
                    generated_header: true,
                });
            }
        }

        // Generate test files per category.
        for group in groups {
            let active: Vec<&Fixture> = group
                .fixtures
                .iter()
                .filter(|f| super::should_include_fixture(f, lang, e2e_config))
                .collect();

            if active.is_empty() {
                continue;
            }

            let filename = format!("{}_test.go", sanitize_filename(&group.category));
            let content = render_test_file(
                &group.category,
                &active,
                GoTestFileContext {
                    go_module_path: &module_path,
                    import_alias: &import_alias,
                    e2e_config,
                    adapters: &config.adapters,
                    data_enum_names: &data_enum_names,
                    config,
                    type_defs,
                    enums,
                },
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
    let _ = writeln!(out, "module {go_module_path}/e2e");
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

/// Generate `main_test.go` that spawns the app harness subprocess before all tests run.
///
/// The harness is expected at `cmd/harness/main.go` (built as `./harness` binary).
/// The harness prints `Harness listening on HOST:PORT` on stdout; we poll TCP port availability.
/// On readiness, we export SUT_URL so all test files can call `os.Getenv("SUT_URL")`.
fn render_main_test_go(test_documents_dir: &str, needs_mock_server_bootstrap: bool, has_http_fixtures: bool) -> String {
    // NOTE: the generated-file header is injected by the caller (generated_header: true).
    let mut out = String::new();
    let _ = writeln!(out, "package e2e_test");
    let _ = writeln!(out);
    let _ = writeln!(out, "import (");
    if needs_mock_server_bootstrap {
        let _ = writeln!(out, "\t\"bufio\"");
        let _ = writeln!(out, "\t\"encoding/json\"");
    }
    let _ = writeln!(out, "\t\"os\"");
    // Only import os/exec if we need to spawn a process (mock-server or harness).
    if needs_mock_server_bootstrap || has_http_fixtures {
        let _ = writeln!(out, "\t\"os/exec\"");
    }
    let _ = writeln!(out, "\t\"path/filepath\"");
    let _ = writeln!(out, "\t\"runtime\"");
    let _ = writeln!(out, "\t\"testing\"");
    if needs_mock_server_bootstrap {
        let _ = writeln!(out, "\t\"fmt\"");
        let _ = writeln!(out, "\t\"net/http\"");
        let _ = writeln!(out, "\t\"strings\"");
        let _ = writeln!(out, "\t\"time\"");
    } else if has_http_fixtures {
        // HTTP-fixture harness path: uses fmt, io, net.DialTimeout for readiness polling.
        let _ = writeln!(out, "\t\"fmt\"");
        let _ = writeln!(out, "\t\"io\"");
        let _ = writeln!(out, "\t\"net\"");
        let _ = writeln!(out, "\t\"time\"");
    }
    let _ = writeln!(out, ")");
    let _ = writeln!(out);
    let _ = writeln!(out, "func TestMain(m *testing.M) {{");
    let _ = writeln!(out, "\t_, filename, _, _ := runtime.Caller(0)");
    let _ = writeln!(out, "\tdir := filepath.Dir(filename)");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "\t// Change to the configured test-documents directory (if it exists) so that fixture"
    );
    let _ = writeln!(
        out,
        "\t// file paths like \"pdf/fake_memo.pdf\" resolve correctly when running go test"
    );
    let _ = writeln!(
        out,
        "\t// from e2e/go/. Repos without document fixtures skip chdir and run from e2e/go/."
    );
    let _ = writeln!(
        out,
        "\ttestDocumentsDir := filepath.Join(dir, \"..\", \"..\", \"{test_documents_dir}\")"
    );
    let _ = writeln!(
        out,
        "\tif info, err := os.Stat(testDocumentsDir); err == nil && info.IsDir() {{"
    );
    let _ = writeln!(out, "\t\tif err := os.Chdir(testDocumentsDir); err != nil {{");
    let _ = writeln!(out, "\t\t\tpanic(err)");
    let _ = writeln!(out, "\t\t}}");
    let _ = writeln!(out, "\t}}");
    let _ = writeln!(out);
    if needs_mock_server_bootstrap {
        // Mock-server bootstrap path: build + spawn mock-server, parse URL, set env, tear down on exit.
        // Mirrors the ruby/elixir/c/dart patterns. Two execution modes:
        //   1. External: MOCK_SERVER_URL already set (alef test-apps run parent).
        //   2. Standalone: build + spawn mock-server, parse URL, set env, tear down on exit.
        let _ = writeln!(out, "\tif os.Getenv(\"MOCK_SERVER_URL\") != \"\" {{");
        let _ = writeln!(out, "\t\tos.Exit(m.Run())");
        let _ = writeln!(out, "\t}}");
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "\tmockBin := filepath.Join(dir, \"..\", \"rust\", \"target\", \"release\", \"mock-server\")"
        );
        let _ = writeln!(
            out,
            "\tmockManifest := filepath.Join(dir, \"..\", \"rust\", \"Cargo.toml\")"
        );
        let _ = writeln!(out, "\tif _, err := os.Stat(mockBin); os.IsNotExist(err) {{");
        let _ = writeln!(out, "\t\tfmt.Fprintln(os.Stderr, \"Building mock-server...\")");
        let _ = writeln!(
            out,
            "\t\tbuild := exec.Command(\"cargo\", \"build\", \"--release\", \"--manifest-path\", mockManifest, \"--bin\", \"mock-server\")"
        );
        let _ = writeln!(out, "\t\tbuild.Stdout = os.Stderr");
        let _ = writeln!(out, "\t\tbuild.Stderr = os.Stderr");
        let _ = writeln!(out, "\t\tif err := build.Run(); err != nil {{");
        let _ = writeln!(out, "\t\t\tpanic(fmt.Sprintf(\"mock-server build failed: %v\", err))");
        let _ = writeln!(out, "\t\t}}");
        let _ = writeln!(out, "\t}}");
        let _ = writeln!(out);
        let _ = writeln!(out, "\tfixturesDir := filepath.Join(dir, \"..\", \"..\", \"fixtures\")");
        let _ = writeln!(out, "\tcmd := exec.Command(mockBin, fixturesDir)");
        let _ = writeln!(
            out,
            "\tcmd.Env = append(os.Environ(), \"MOCK_SERVER_NO_STDIN_WATCH=1\")"
        );
        let _ = writeln!(out, "\tstdout, err := cmd.StdoutPipe()");
        let _ = writeln!(out, "\tif err != nil {{ panic(err) }}");
        let _ = writeln!(out, "\tcmd.Stderr = os.Stderr");
        let _ = writeln!(out, "\tif err := cmd.Start(); err != nil {{ panic(err) }}");
        let _ = writeln!(
            out,
            "\t// Defer covers panics during bootstrap (scanner / readiness poll)."
        );
        let _ = writeln!(
            out,
            "\t// The happy path explicitly kills before `os.Exit` below — without"
        );
        let _ = writeln!(
            out,
            "\t// that, `os.Exit` skips this defer, leaves the child running, and"
        );
        let _ = writeln!(
            out,
            "\t// `go test` reports \"Test I/O incomplete\" / WaitDelay expired."
        );
        let _ = writeln!(out, "\tdefer func() {{ _ = cmd.Process.Kill() }}()");
        let _ = writeln!(out);
        let _ = writeln!(out, "\tscanner := bufio.NewScanner(stdout)");
        let _ = writeln!(out, "\tscanner.Buffer(make([]byte, 0, 64*1024), 1024*1024)");
        let _ = writeln!(
            out,
            "\t// The mock-server emits two sentinel lines on stdout: MOCK_SERVER_URL=<url>"
        );
        let _ = writeln!(
            out,
            "\t// (always) and MOCK_SERVERS={{\"<fixture_id>\":\"<per-fixture-url>\",...}} (when"
        );
        let _ = writeln!(
            out,
            "\t// any fixture has origin-root routes that need a per-fixture listener). We"
        );
        let _ = writeln!(
            out,
            "\t// read until we have seen MOCK_SERVER_URL and either MOCK_SERVERS or a non"
        );
        let _ = writeln!(out, "\t// MOCK_SERVER line, then drain the rest in the background.");
        let _ = writeln!(out, "\thaveURL := false");
        let _ = writeln!(out, "\t//nolint:gocritic");
        let _ = writeln!(out, "\tfor scanner.Scan() {{");
        let _ = writeln!(out, "\t\tline := scanner.Text()");
        let _ = writeln!(out, "\t\tif strings.HasPrefix(line, \"MOCK_SERVER_URL=\") {{");
        let _ = writeln!(
            out,
            "\t\t\t_ = os.Setenv(\"MOCK_SERVER_URL\", strings.TrimPrefix(line, \"MOCK_SERVER_URL=\"))"
        );
        let _ = writeln!(out, "\t\t\thaveURL = true");
        let _ = writeln!(out, "\t\t\tcontinue");
        let _ = writeln!(out, "\t\t}} else if strings.HasPrefix(line, \"MOCK_SERVERS=\") {{");
        let _ = writeln!(out, "\t\t\tpayload := strings.TrimPrefix(line, \"MOCK_SERVERS=\")");
        let _ = writeln!(out, "\t\t\t_ = os.Setenv(\"MOCK_SERVERS\", payload)");
        let _ = writeln!(out, "\t\t\tvar servers map[string]string");
        let _ = writeln!(
            out,
            "\t\t\tif err := json.Unmarshal([]byte(payload), &servers); err == nil {{"
        );
        let _ = writeln!(out, "\t\t\t\tfor fid, furl := range servers {{");
        let _ = writeln!(
            out,
            "\t\t\t\t\t_ = os.Setenv(\"MOCK_SERVER_\"+strings.ToUpper(fid), furl)"
        );
        let _ = writeln!(out, "\t\t\t\t}}");
        let _ = writeln!(out, "\t\t\t}}");
        let _ = writeln!(out, "\t\t\tbreak");
        let _ = writeln!(out, "\t\t}} else if haveURL {{");
        let _ = writeln!(out, "\t\t\tbreak");
        let _ = writeln!(out, "\t\t}}");
        let _ = writeln!(out, "\t}}");
        let _ = writeln!(out, "\tif os.Getenv(\"MOCK_SERVER_URL\") == \"\" {{");
        let _ = writeln!(out, "\t\tpanic(\"mock-server did not emit MOCK_SERVER_URL\")");
        let _ = writeln!(out, "\t}}");
        let _ = writeln!(
            out,
            "\t// Drain remaining stdout asynchronously so the pipe doesn't fill."
        );
        let _ = writeln!(out, "\tgo func() {{ for scanner.Scan() {{ }} }}()");
        let _ = writeln!(out);
        // Wait until the mock-server actually accepts a TCP connection on the
        // URL it just announced. The `MOCK_SERVER_URL=` sentinel is printed by
        // the rust binary right after listener.bind() succeeds — the kernel
        // queues SYNs from that point on — but the axum::serve task is
        // spawned only afterward and may not have started accept()ing yet
        // when the first test request fires. A 1-second connect-poll closes
        // that window without the panic the previous code would have hit if
        // the connect ever blocked.
        let _ = writeln!(
            out,
            "\t// Poll the mock-server URL until it answers (axum::serve start race)."
        );
        let _ = writeln!(out, "\t{{");
        let _ = writeln!(out, "\t\turl := os.Getenv(\"MOCK_SERVER_URL\")");
        let _ = writeln!(out, "\t\tready := false");
        let _ = writeln!(out, "\t\tfor i := 0; i < 400; i++ {{");
        let _ = writeln!(out, "\t\t\tresp, err := http.Get(url)");
        let _ = writeln!(out, "\t\t\tif err == nil {{");
        let _ = writeln!(out, "\t\t\t\t_ = resp.Body.Close()");
        let _ = writeln!(out, "\t\t\t\tready = true");
        let _ = writeln!(out, "\t\t\t\tbreak");
        let _ = writeln!(out, "\t\t\t}}");
        let _ = writeln!(out, "\t\t\ttime.Sleep(50 * time.Millisecond)");
        let _ = writeln!(out, "\t\t}}");
        let _ = writeln!(out, "\t\tif !ready {{");
        let _ = writeln!(out, "\t\t\tpanic(\"mock-server did not become ready within 20s\")");
        let _ = writeln!(out, "\t\t}}");
        let _ = writeln!(out, "\t}}");
        let _ = writeln!(out);
        let _ = writeln!(out, "\tcode := m.Run()");
        let _ = writeln!(
            out,
            "\t// Kill the mock-server BEFORE os.Exit so the child stops writing to"
        );
        let _ = writeln!(
            out,
            "\t// the stderr pipe inherited from the test process. Without this the"
        );
        let _ = writeln!(out, "\t// Go test runner waits for the pipe to close and reports");
        let _ = writeln!(out, "\t// \"exec: WaitDelay expired before I/O complete\".");
        let _ = writeln!(out, "\t_ = cmd.Process.Kill()");
        let _ = writeln!(out, "\t_, _ = cmd.Process.Wait()");
        let _ = writeln!(out, "\tos.Exit(code)");
        let _ = writeln!(out, "}}");
        return out;
    }
    // Harness-spawn path: only emit when HTTP fixtures are present.
    if !has_http_fixtures {
        // No mock-server bootstrap and no HTTP fixtures: just exit after chdir.
        let _ = writeln!(out, "\tos.Exit(m.Run())");
        let _ = writeln!(out, "}}");
        return out;
    }
    let _ = writeln!(
        out,
        "\t// If SUT_URL is already set, a parent process started a shared harness."
    );
    let _ = writeln!(out, "\t// Use it as-is and do NOT spawn our own.");
    let _ = writeln!(out, "\tif os.Getenv(\"SUT_URL\") != \"\" {{");
    let _ = writeln!(out, "\t\tos.Exit(m.Run())");
    let _ = writeln!(out, "\t}}");
    let _ = writeln!(out);
    let _ = writeln!(out, "\t// Spawn the harness executable.");
    let _ = writeln!(
        out,
        "\tharnessBin := filepath.Join(dir, \"cmd\", \"harness\", \"harness\")"
    );
    let _ = writeln!(out, "\tcmd := exec.Command(harnessBin)");
    let _ = writeln!(out, "\tcmd.Stderr = os.Stderr");
    let _ = writeln!(out, "\t// Keep pipes open so harness doesn't exit immediately.");
    let _ = writeln!(out, "\tstdin, err := cmd.StdinPipe()");
    let _ = writeln!(out, "\tif err != nil {{");
    let _ = writeln!(out, "\t\tpanic(fmt.Sprintf(\"stdin pipe: %v\", err))");
    let _ = writeln!(out, "\t}}");
    let _ = writeln!(out, "\tstdout, err := cmd.StdoutPipe()");
    let _ = writeln!(out, "\tif err != nil {{");
    let _ = writeln!(out, "\t\tpanic(fmt.Sprintf(\"stdout pipe: %v\", err))");
    let _ = writeln!(out, "\t}}");
    let _ = writeln!(out, "\tif err := cmd.Start(); err != nil {{");
    let _ = writeln!(out, "\t\tpanic(fmt.Sprintf(\"start harness: %v\", err))");
    let _ = writeln!(out, "\t}}");
    let _ = writeln!(out);
    let _ = writeln!(out, "\t// Poll TCP port 8012 until harness is ready (15s timeout).");
    let _ = writeln!(out, "\thost := \"127.0.0.1\"");
    let _ = writeln!(out, "\tport := \"8012\"");
    let _ = writeln!(out, "\tsutURL := \"http://\" + host + \":\" + port");
    let _ = writeln!(out, "\tdeadline := time.Now().Add(15 * time.Second)");
    let _ = writeln!(out, "\tfor time.Now().Before(deadline) {{");
    let _ = writeln!(
        out,
        "\t\tconn, err := net.DialTimeout(\"tcp\", host+\":\"+port, 500*time.Millisecond)"
    );
    let _ = writeln!(out, "\t\tif err == nil {{");
    let _ = writeln!(out, "\t\t\tconn.Close()");
    let _ = writeln!(out, "\t\t\tbreak");
    let _ = writeln!(out, "\t\t}}");
    let _ = writeln!(out, "\t\tif cmd.ProcessState != nil {{");
    let _ = writeln!(out, "\t\t\t// Harness exited early.");
    let _ = writeln!(out, "\t\t\tstderr, _ := io.ReadAll(os.Stderr)");
    let _ = writeln!(out, "\t\t\tpanic(fmt.Sprintf(\"harness died: %s\", stderr))");
    let _ = writeln!(out, "\t\t}}");
    let _ = writeln!(out, "\t\ttime.Sleep(100 * time.Millisecond)");
    let _ = writeln!(out, "\t}}");
    let _ = writeln!(out);
    let _ = writeln!(out, "\tos.Setenv(\"SUT_URL\", sutURL)");
    let _ = writeln!(out, "\t// Drain stdout so the pipe doesn't block.");
    let _ = writeln!(out, "\tgo func() {{ _, _ = io.Copy(io.Discard, stdout) }}()");
    let _ = writeln!(out);
    let _ = writeln!(out, "\tcode := m.Run()");
    let _ = writeln!(out);
    let _ = writeln!(out, "\t// Cleanup: close stdin and wait for harness.");
    let _ = writeln!(out, "\t_ = stdin.Close()");
    let _ = writeln!(out, "\t_ = cmd.Process.Signal(os.Interrupt)");
    let _ = writeln!(out, "\t_ = cmd.Wait()");
    let _ = writeln!(out);
    let _ = writeln!(out, "\tos.Exit(code)");
    let _ = writeln!(out, "}}");
    out
}

/// Generate `helpers_test.go` with the jsonString helper function.
/// This is emitted once per package to avoid duplicate function definitions.
fn render_helpers_test_go() -> String {
    let mut out = String::new();
    let _ = writeln!(out, "package e2e_test");
    let _ = writeln!(out);
    let _ = writeln!(out, "import \"encoding/json\"");
    let _ = writeln!(out);
    let _ = writeln!(out, "// jsonString converts a value to its JSON string representation.");
    let _ = writeln!(
        out,
        "// Array fields use jsonString instead of fmt.Sprint to preserve structure."
    );
    let _ = writeln!(out, "//nolint:unused");
    let _ = writeln!(out, "func jsonString(value any) string {{");
    let _ = writeln!(out, "\tencoded, err := json.Marshal(value)");
    let _ = writeln!(out, "\tif err != nil {{");
    let _ = writeln!(out, "\t\treturn \"\"");
    let _ = writeln!(out, "\t}}");
    let _ = writeln!(out, "\treturn string(encoded)");
    let _ = writeln!(out, "}}");
    out
}

/// Generate `cmd/harness/main.go` — the app harness that serves fixtures for server-pattern e2e tests.
fn render_harness_main(_e2e_config: &E2eConfig, groups: &[FixtureGroup], go_module_path: &str) -> String {
    use minijinja::{Environment, context};

    // Collect all HTTP fixtures into a fixtures map JSON.
    let mut fixtures_map = serde_json::Map::new();
    for group in groups {
        for fixture in &group.fixtures {
            if fixture.http.is_none() {
                continue;
            }
            let http_data = fixture.http.as_ref().unwrap();
            let fixture_json = serde_json::json!({
                "http": {
                    "handler": {
                        "route": &http_data.handler.route,
                        "method": &http_data.handler.method,
                        "body_schema": http_data.handler.body_schema.clone(),
                    },
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
    let fixtures_json_obj = serde_json::Value::Object(fixtures_map);
    let fixtures_json_str = serde_json::to_string(&fixtures_json_obj).unwrap_or_default();
    // Escape the JSON string for use in a Go quoted string.
    // Must escape backslashes first, then double quotes.
    let fixtures_json = fixtures_json_str.replace('\\', "\\\\").replace('"', "\\\"");

    // Render via Jinja template.
    let mut env = Environment::new();
    let harness_template = include_str!("../templates/go/harness_main.go.jinja");
    env.add_template("harness", harness_template).ok();

    // Derive a short import alias from the module path (e.g., the last path segment).
    let import_alias = go_module_path.rsplit('/').next().unwrap_or("pkg").to_string();

    let template = env.get_template("harness").unwrap();
    let output = template
        .render(context! {
            imports => vec![go_module_path],
            import_alias => import_alias,
            register_route_method => "RegisterRoute",
            run_method => "Run",
            start_background_method => "StartBackground",
            port => 8012,
            fixtures_json => fixtures_json,
        })
        .unwrap_or_default();

    // Prepend the generated-file header.
    let mut out = hash::header(CommentStyle::DoubleSlash);
    out.push_str(&output);
    out
}

mod assertions;
mod json_values;
mod method_calls;
mod setup;
mod test_backend;
mod test_file;
mod test_function;
#[cfg(test)]
mod tests;
mod visitors;

pub use test_backend::emit_test_backend;
use test_file::{GoTestFileContext, render_test_file};
use test_function::fixture_has_go_callable;
