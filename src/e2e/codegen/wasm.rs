//! WebAssembly e2e test generator using vitest.
//!
//! Reuses the TypeScript test renderer for both HTTP and non-HTTP fixtures,
//! configured with the generated WASM package as the import
//! path and `wasm` as the language key for skip/override resolution. Adds
//! wasm-specific scaffolding: a `setup.ts` chdir to `test_documents/` so
//! file_path fixtures resolve, and a `globalSetup.ts` that spawns the
//! app harness (server-pattern) for HTTP fixtures. The wasm-pack `--target nodejs`
//! CJS bundle initializes synchronously and does not require vite-plugin-wasm.

use crate::e2e::config::E2eConfig;
use crate::e2e::escape::sanitize_filename;

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::hash::{self, CommentStyle};
use crate::core::template_versions as tv;
use crate::e2e::fixture::{Fixture, FixtureGroup};
use anyhow::Result;
use std::fmt::Write as FmtWrite;
use std::path::PathBuf;

use super::E2eCodegen;
use super::typescript::config::render_global_setup;

/// WebAssembly e2e code generator.
pub struct WasmCodegen;

impl E2eCodegen for WasmCodegen {
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
        let tests_base = output_base.join("tests");

        let mut files = Vec::new();

        // Resolve call config with wasm-specific overrides.
        let call = &e2e_config.call;
        let overrides = call.overrides.get(lang);
        let module_path = overrides
            .and_then(|o| o.module.as_ref())
            .cloned()
            .unwrap_or_else(|| call.module.clone());
        let function_name = overrides
            .and_then(|o| o.function.as_ref())
            .cloned()
            .unwrap_or_else(|| snake_to_camel(&call.function));
        let client_factory = overrides.and_then(|o| o.client_factory.as_deref());

        // Resolve package config — defaults to a co-located pkg/ directory shipped
        // by `wasm-pack build` next to the wasm crate.
        // When `[crates.output] wasm` is set explicitly, derive the pkg path from
        // that value so that renamed WASM crates resolve correctly without any
        // hardcoded special cases.
        let wasm_pkg = e2e_config.resolve_package("wasm");
        // `pkg_path_is_explicit` distinguishes "user told us exactly where the
        // npm-consumable package lives" from "fall back to the default
        // wasm-pack output directory". The render below appends `/nodejs` only
        // for the fallback case (`wasm_crate_path()` returns the crate's
        // `pkg/` dir, whose npm-consumable subpackage is at `pkg/nodejs/`).
        // For an explicit path the user is responsible for pointing at a
        // directory that already has a valid `package.json`.
        let (pkg_path, pkg_path_is_explicit) = match wasm_pkg.as_ref().and_then(|p| p.path.as_ref()) {
            Some(p) => (p.clone(), true),
            None => (config.wasm_crate_path(), false),
        };
        let pkg_name = wasm_pkg
            .as_ref()
            .and_then(|p| p.name.as_ref())
            .cloned()
            .unwrap_or_else(|| config.wasm_package_name());
        let pkg_version = wasm_pkg
            .as_ref()
            .and_then(|p| p.version.as_ref())
            .cloned()
            .or_else(|| config.resolved_version())
            .unwrap_or_else(|| "0.1.0".to_string());

        // Determine which auxiliary scaffolding files we need based on the active
        // fixture set. Doing this once up front lets us emit a self-contained vitest
        // config that wires only the setup files we'll actually generate.
        //
        // WASM language filtering: when `[crates.wasm].languages` is set, auto-skip
        // fixtures for languages not in that static-compiled list. This bridges the gap
        // between the full language pack and WASM's 8-language static build.
        let wasm_languages = config.wasm.as_ref().and_then(|w| {
            if w.languages.is_empty() {
                None
            } else {
                Some(w.languages.clone())
            }
        });

        // Build active fixtures per group. For WASM, when the backend declares a static
        // language set via `[crates.wasm].languages`, include fixtures for languages
        // not in that set but mark them with auto-skip directives so they render as
        // `it.skip()` tests instead of being omitted entirely.
        let active_per_group: Vec<Vec<Fixture>> = groups
            .iter()
            .map(|group| {
                let mut result = Vec::new();
                for fixture in &group.fixtures {
                    // Determine if this fixture should be included.
                    // Start with the base should_include_fixture check.
                    let mut base_include = super::should_include_fixture(fixture, lang, e2e_config);

                    // When `[crates.wasm].languages` is set, force `base_include = false` for
                    // any fixture whose `input.language` falls outside that static-compiled set.
                    // The else-branch below will then attach an auto-skip directive so the test
                    // renders as `it.skip(...)` rather than running against a missing grammar.
                    // `should_include_fixture` does not inspect `input.language`, so without this
                    // override fixtures like `{ input: { language: "abl" } }` (where "abl" is not
                    // in the wasm bundle) would be emitted normally and fail at runtime.
                    if base_include {
                        if let Some(ref wasm_langs) = wasm_languages {
                            // Look for the target grammar in either of the two shapes
                            // alef fixtures use: top-level `input.language` (function-call
                            // shape) or nested `input.config.language` (config-object shape
                            // used by smoke fixtures and anything taking a typed config DTO).
                            let fix_lang = fixture.input.get("language").and_then(|v| v.as_str()).or_else(|| {
                                fixture
                                    .input
                                    .get("config")
                                    .and_then(|c| c.get("language"))
                                    .and_then(|v| v.as_str())
                            });
                            if let Some(fix_lang) = fix_lang {
                                if !wasm_langs.iter().any(|l| l == fix_lang) {
                                    base_include = false;
                                }
                            }
                        }
                    }

                    // Check per-call skip_languages (fixture is completely unsupported)
                    let cc = e2e_config.resolve_call_for_fixture(
                        fixture.call.as_deref(),
                        &fixture.id,
                        &fixture.resolved_category(),
                        &fixture.tags,
                        &fixture.input,
                    );
                    if cc.skip_languages.iter().any(|l| l == lang) {
                        // Per-call skip — drop entirely, never include
                        continue;
                    }

                    if base_include {
                        // Check node fetch compatibility
                        if let Some(http) = &fixture.http {
                            if http
                                .request
                                .headers
                                .iter()
                                .any(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                            {
                                // Node fetch rejects mismatched Content-Length — skip fixture
                                continue;
                            }
                            let m = http.request.method.to_ascii_uppercase();
                            if m == "TRACE" || m == "CONNECT" {
                                // Node fetch doesn't support these methods — skip fixture
                                continue;
                            }
                        }

                        // Include the fixture normally
                        result.push(fixture.clone());
                    } else {
                        // Fixture failed should_include_fixture or language not in wasm set.
                        // Omit entirely — do not emit as it.skip().
                        continue;
                    }
                }
                result
            })
            .collect();

        let any_fixtures = active_per_group.iter().flat_map(|g| g.iter());
        // The wasm globalSetup spawns the mock server. It must run for any fixture
        // that interpolates `${process.env.MOCK_SERVER_URL}` into a base URL —
        // i.e. anything with `mock_response` or `http`, not just raw
        // `is_http_test`. The comment block below this line states the same
        // intent; the previous condition (`f.is_http_test()`) only detected
        // the `http: { ... }` shape and missed direct mock-response fixtures.
        let has_http_fixtures = any_fixtures.clone().any(|f| f.needs_mock_server());
        // file_path / bytes args are read off disk by the generated code at runtime;
        // we add a setup.ts chdir to test_documents so relative paths resolve.
        let has_file_fixtures = active_per_group.iter().flatten().any(|f| {
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

        // Generate package.json — adds vitest + rollup dev deps so that the test
        // suite can import the wasm-pack nodejs CJS bundle by package name.
        files.push(GeneratedFile {
            path: output_base.join("package.json"),
            content: render_package_json(
                &pkg_name,
                &pkg_path,
                pkg_path_is_explicit,
                &pkg_version,
                e2e_config.dep_mode,
                e2e_config.harness_extras.get("wasm"),
            ),
            generated_header: false,
        });

        // Generate vitest.config.ts — globalSetup is needed for any fixture that
        // interpolates `${process.env.MOCK_SERVER_URL}` (`mock_response` or `http`
        // shape — both produce `has_http_fixtures`
        // via `Fixture::needs_mock_server`). The simple mock-server template spawns
        // the standalone `mock-server` binary; the server-pattern template spawns
        // the consumer's app harness. Selection mirrors the Node typescript codegen.
        //
        // For wasm, the service API is skipped (App class is excluded from bindings).
        // Wasm fixtures that require an app harness cannot run; fall back to mock-server
        // pattern. Check if the app_class is in the wasm exclude_types.
        let app_class_excluded = config
            .wasm
            .as_ref()
            .map(|w| w.exclude_types.iter().any(|t| t == "App"))
            .unwrap_or(false);
        let use_server_pattern = has_http_fixtures && !e2e_config.harness.imports.is_empty() && !app_class_excluded;
        let needs_global_setup = has_http_fixtures;
        let with_file_setup_cfg = has_file_fixtures || has_http_fixtures;
        files.push(GeneratedFile {
            path: output_base.join("vitest.config.ts"),
            content: render_vitest_config(needs_global_setup, with_file_setup_cfg),
            generated_header: true,
        });

        // The server-pattern `app_harness.mjs` (SUT-as-server) is delegated to a
        // consumer extension via `Extension::emit_e2e`; alef no longer emits it.

        // Generate the mock-server globalSetup.ts. The server-pattern variant
        // (which spawns the consumer's app_harness subprocess) is delegated to an
        // extension; alef emits only the standalone `mock-server` globalSetup, so
        // skip it here when the server pattern is active (the extension owns it).
        if needs_global_setup && !use_server_pattern {
            files.push(GeneratedFile {
                path: output_base.join("globalSetup.ts"),
                content: render_global_setup(false),
                generated_header: true,
            });
        }

        // Generate setup.ts — runs per-test-worker via vitest setupFiles.
        // When file fixtures are present: chdir to test_documents/ so relative
        // fixture paths resolve, plus the wasm module init block.
        // When only HTTP fixtures are present: emit just the wasm init block.
        // The wasm init MUST run in setupFiles (per-worker), not just in
        // globalSetup (main process), because each vitest worker gets a fresh
        // module graph; init done in globalSetup does not propagate to workers.
        let needs_setup_ts = has_file_fixtures || has_http_fixtures;
        if needs_setup_ts {
            files.push(GeneratedFile {
                path: output_base.join("setup.ts"),
                content: render_setup(
                    &e2e_config.test_documents_dir,
                    has_file_fixtures,
                    &pkg_name,
                    &e2e_config.env,
                ),
                generated_header: true,
            });
        }

        // Generate tsconfig.json — prevents Vite from walking up to a project-level
        // tsconfig and pulling in unrelated compiler options.
        files.push(GeneratedFile {
            path: output_base.join("tsconfig.json"),
            content: render_tsconfig(),
            generated_header: false,
        });

        // Emit a local `pnpm-workspace.yaml` declaring `e2e/wasm/` as its own
        // pnpm workspace root. Without this, `pnpm install` walks up to the
        // repo-root `pnpm-workspace.yaml`, where polyglot repos commonly
        // exclude `e2e/wasm` (it depends on a `wasm-pack build` artifact that
        // is absent on fresh checkouts). The CLI flag `--ignore-workspace`
        // would also work, but it forces every caller (Taskfile, CI step) to
        // pass it; making `e2e/wasm/` self-rooted keeps the generated suite
        // self-contained.
        // `allowBuilds` opts native-build scripts of common transitive deps
        // (`esbuild`, `tree-sitter`) in. pnpm 11 refuses to silently run
        // postinstall scripts and fails with `ERR_PNPM_IGNORED_BUILDS` unless
        // they are listed explicitly.
        //
        // In Registry mode (test_apps/), also emit `minimumReleaseAgeExclude`
        // pinning the just-published wasm package version so pnpm 11.3+'s
        // supply-chain freshness gate does not reject installation of the
        // package under test. The gate rejects packages younger than the
        // configured minimum release age (default 24h); the
        // `minimumReleaseAgeExclude` allowlist exempts the specific version.
        let workspace_yaml_content = {
            let mut content =
                String::from("packages:\n  - \".\"\nallowBuilds:\n  esbuild: true\n  tree-sitter: true\n");
            if e2e_config.dep_mode == crate::e2e::config::DependencyMode::Registry {
                use std::fmt::Write as _;
                // minimumReleaseAgeExclude expects a concrete `name@version`
                // identifier; strip any semver constraint operator (`^`, `~`,
                // `>`, `<`, `=`) that may be carried over from the
                // `[crates.e2e.registry.packages.<lang>] version` pin.
                let bare_version = pkg_version.trim_start_matches(['^', '~', '>', '<', '=']);
                let _ = write!(content, "minimumReleaseAgeExclude:\n  - '{pkg_name}@{bare_version}'\n");
            }
            content
        };
        files.push(GeneratedFile {
            path: output_base.join("pnpm-workspace.yaml"),
            content: workspace_yaml_content,
            // `generated_header: true` so the cleanup pass can recognize and
            // sweep this file if the wasm e2e codegen ever stops emitting it
            // (or if consumers migrate to a different workspace layout).
            // Symmetric with the typescript codegen's pnpm-workspace.yaml.
            // YAML handles `#` comments fine; pnpm's parser preserves them.
            generated_header: true,
        });

        // Resolve options_type from override (e.g. `WasmExtractionConfig`).
        let options_type = overrides.and_then(|o| o.options_type.clone());

        // Generate test files per category. We delegate the per-fixture rendering
        // to the typescript codegen (`render_test_file`), which already handles
        // both HTTP and function-call fixtures correctly. Passing `lang = "wasm"`
        // routes per-fixture override resolution and skip checks through the wasm
        // language key. We then inject Node.js WASM initialization code to load
        // the WASM binary from the pkg directory using fs.readFileSync.
        let wasm_type_prefix = config.wasm_type_prefix();
        for (group, active) in groups.iter().zip(active_per_group.iter()) {
            if active.is_empty() {
                continue;
            }
            let filename = format!("{}.test.ts", sanitize_filename(&group.category));
            // Convert Vec<Fixture> to Vec<&Fixture> for render_test_file
            let active_refs: Vec<&Fixture> = active.iter().collect();
            let content = super::typescript::render_test_file(
                lang,
                &group.category,
                &active_refs,
                &module_path,
                &pkg_name,
                &function_name,
                &e2e_config.call.args,
                options_type.as_deref(),
                client_factory,
                e2e_config,
                type_defs,
                enums,
                &wasm_type_prefix,
                config,
            );

            // A category can survive the `active.is_empty()` guard above yet still render
            // to a bare `describe(...)` with no cases when every fixture in it is dropped by
            // the typescript renderer (e.g. websocket fixtures, which the wasm binding cannot
            // exercise). vitest fails such a file with "No test found in suite", so skip
            // emitting it when no `it`/`test` case was produced.
            if !content.contains("it(") && !content.contains("it.skip(") && !content.contains("test(") {
                continue;
            }

            // The local `pkg/` directory produced by `wasm-pack build --target nodejs`
            // is already a Node-friendly self-initializing CJS module — `pkg/package.json`
            // sets `"main"` to the JS entry, so test files can import the package by name
            // (`from "<pkg_name>"`) with no subpath. The historical `dist-node` rewrite
            // assumed a multi-distribution layout (`dist/`, `dist-node/`, `dist-web/`)
            // that the alef-managed `wasm-pack build` does not produce; it is therefore
            // intentionally absent here.
            let _ = (&pkg_path, &config.name); // keep variables alive for future use

            files.push(GeneratedFile {
                path: tests_base.join(filename),
                content,
                generated_header: true,
            });
        }

        // Registry-mode test_apps/ runners (e.g. a consumer's
        // `task smoke:wasm` step) invoke a fixed `pnpm exec vitest run
        // tests/smoke.test.ts` smoke target by convention. Emit a minimal
        // smoke test file whenever no `smoke` fixture category is present
        // so the runner does not error on a missing path.
        //
        // The emitted file imports the published wasm package and asserts
        // that the module entry resolves — a true smoke test that catches
        // packaging regressions (missing `main`/`exports`, wasm-init
        // failures) without depending on any specific binding API.
        if e2e_config.dep_mode == crate::e2e::config::DependencyMode::Registry {
            let smoke_path = tests_base.join("smoke.test.ts");
            let has_smoke_emitted = files.iter().any(|f| f.path == smoke_path);
            if !has_smoke_emitted {
                files.push(GeneratedFile {
                    path: smoke_path,
                    content: render_wasm_smoke_test(&pkg_name),
                    generated_header: true,
                });
            }
        }

        Ok(files)
    }

    fn language_name(&self) -> &'static str {
        "wasm"
    }
}

/// Render a minimal smoke test importing the published wasm package.
///
/// The test asserts the module imports cleanly — a regression here points
/// at a packaging fault (missing entry, broken wasm-init, ESM/CJS export
/// mismatch) rather than a binding-API issue.
fn render_wasm_smoke_test(pkg_name: &str) -> String {
    format!(
        r#"import {{ describe, expect, it }} from "vitest";
import * as pkg from "{pkg_name}";

describe("smoke", () => {{
    it("imports the published wasm package", () => {{
        expect(pkg).toBeDefined();
        expect(typeof pkg).toBe("object");
    }});
}});
"#
    )
}

fn snake_to_camel(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut upper_next = false;
    for ch in s.chars() {
        if ch == '_' {
            upper_next = true;
        } else if upper_next {
            out.push(ch.to_ascii_uppercase());
            upper_next = false;
        } else {
            out.push(ch);
        }
    }
    out
}

fn render_package_json(
    pkg_name: &str,
    pkg_path: &str,
    pkg_path_is_explicit: bool,
    pkg_version: &str,
    dep_mode: crate::e2e::config::DependencyMode,
    extras: Option<&crate::core::config::manifest_extras::ManifestExtras>,
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
        // Fallback path: `wasm-pack build --target nodejs --out-dir pkg/nodejs` writes
        // the npm-consumable package (its own package.json with `main`/`types` etc.)
        // to `pkg/nodejs/`, not to `pkg/` directly. The fallback `wasm_crate_path()`
        // points at `pkg/`, so we descend into `nodejs/` to find a valid
        // package.json. When the user has set `[e2e.packages.wasm].path` explicitly,
        // we trust they have already pointed at a directory with a valid package.json
        // (the crate root, the wasm-pack out-dir, or another distribution layout) and
        // do not mutate it.
        crate::e2e::config::DependencyMode::Local => {
            if pkg_path_is_explicit {
                format!("file:{pkg_path}")
            } else {
                format!("file:{pkg_path}/nodejs")
            }
        }
    };
    let rendered = crate::e2e::template_env::render(
        "wasm/package.json.jinja",
        minijinja::context! {
            pkg_name => pkg_name,
            dep_value => dep_value,
            rollup => tv::npm::ROLLUP,
            vitest => tv::npm::VITEST,
            node_engine => tv::npm::NODE_ENGINE,
        },
    );
    match extras {
        Some(e) if !e.is_empty() => crate::e2e::codegen::typescript::config::inject_package_json_extras(&rendered, e),
        _ => rendered,
    }
}

fn render_vitest_config(with_global_setup: bool, with_file_setup: bool) -> String {
    let header = hash::header(CommentStyle::DoubleSlash);
    crate::e2e::template_env::render(
        "wasm/vitest.config.ts.jinja",
        minijinja::context! {
            header => header,
            with_global_setup => with_global_setup,
            with_file_setup => with_file_setup,
        },
    )
}

/// Render `setup.ts` — vitest `setupFiles` entry, runs per test worker.
///
/// Always emits the wasm-bindgen async `init()` call so that the wasm
/// module is fully instantiated before any test body runs. This MUST live
/// in `setupFiles` (per-worker), not in `globalSetup` (main process only),
/// because each vitest worker spawns its own module graph; init done in
/// the main process does not propagate.
///
/// When `include_file_setup` is true, also patches the CommonJS loader for
/// wasm-pack `--target nodejs` WASI/env stubs and chdir's to
/// `test_documents_dir` so file-path fixture arguments resolve.
fn render_setup(
    test_documents_dir: &str,
    include_file_setup: bool,
    pkg_name: &str,
    env: &std::collections::HashMap<String, String>,
) -> String {
    let header = hash::header(CommentStyle::DoubleSlash);
    let mut out = header;

    // Consolidated import block for both wasm init and file setup.
    out.push_str("import { createRequire } from 'module';\n");
    out.push_str("import { readFileSync } from 'fs';\n");
    out.push_str("import { fileURLToPath } from 'url';\n");
    if include_file_setup {
        out.push_str("import { dirname, join } from 'path';\n");
    }
    out.push('\n');

    // Emit e2e env var assignments, alphabetically sorted by key.
    if !env.is_empty() {
        let mut entries: Vec<_> = env.iter().collect();
        entries.sort_by_key(|(k, _)| *k);
        for (key, value) in entries {
            let _ = writeln!(out, "process.env.{key} ??= {value:?};");
        }
        out.push('\n');
    }

    // Wasm module init — must run before any wasm export is called.
    // Published wasm-bindgen packages export `initSync` (synchronous, takes a
    // WebAssembly.Module or ArrayBuffer) and a default async `__wbg_init`.
    // Node.js built-in `fetch` does not support `file://` URLs, so the async
    // default fails when called without a path in a Node.js context. We
    // instead locate the `.wasm` binary via `createRequire`, read it with
    // `readFileSync`, and call `initSync` with the buffer. This is always safe
    // and works in both Node.js test workers and browser contexts.
    // For wasm-pack `--target nodejs` bundles that self-initialize synchronously
    // the import succeeds but `initSync` may be absent — the try/catch is a no-op.
    out.push_str("// Pre-initialize the wasm-bindgen module so that exports are callable\n");
    out.push_str("// in every vitest worker. The async default export uses fetch() which\n");
    out.push_str("// does not support file:// URLs in Node.js; use initSync with a\n");
    out.push_str("// readFileSync buffer instead.\n");
    let _ = writeln!(out, "try {{");
    let _ = writeln!(out, "  const _require = createRequire(import.meta.url);");
    let _ = writeln!(out, "  const wasmPkgDir = _require.resolve('{pkg_name}');");
    let _ = writeln!(out, "  const wasmModule = await import(/* @vite-ignore */ wasmPkgDir);");
    out.push_str("  const initSync = (wasmModule as unknown as Record<string, unknown>).initSync as ((mod: WebAssembly.Module | BufferSource) => unknown) | undefined;\n");
    out.push_str("  if (typeof initSync === 'function') {\n");
    out.push_str("    // Locate the .wasm binary next to the JS entry.\n");
    out.push_str("    const wasmJsPath = fileURLToPath(new URL(wasmPkgDir, 'file://'));\n");
    out.push_str("    const wasmBinPath = wasmJsPath.replace(/\\.js$/, '_bg.wasm');\n");
    out.push_str("    const wasmBytes = readFileSync(wasmBinPath);\n");
    out.push_str("    // Pass as object form to avoid wasm-bindgen deprecation warning.\n");
    out.push_str("    initSync({ module: wasmBytes });\n");
    out.push_str("  } else {\n");
    out.push_str("    // Fallback: try the async default init (wasm-pack --target nodejs bundles).\n");
    out.push_str("    const initDefault = (wasmModule as unknown as Record<string, unknown>).default as (() => Promise<unknown>) | undefined;\n");
    out.push_str("    if (typeof initDefault === 'function') await initDefault();\n");
    out.push_str("  }\n");
    out.push_str("} catch (err) {\n");
    out.push_str("  // Module may not require explicit init — continue anyway.\n");
    out.push_str("  console.warn('[alef wasm setup] init skipped:', (err as Error).message);\n");
    out.push_str("}\n\n");
    if include_file_setup {
        let file_only = render_file_setup(test_documents_dir);
        // render_file_setup prepends its own header; strip the header and import lines
        // (everything before the first comment after imports).
        let body_start = file_only
            .find("// Patch CommonJS")
            .or_else(|| file_only.find("// Change to"))
            .unwrap_or(0);
        out.push_str(&file_only[body_start..]);
    }
    out
}

fn render_file_setup(test_documents_dir: &str) -> String {
    // Note: imports are now consolidated in render_setup() to avoid duplication.
    // This function returns the body content (after imports) for embedding.
    let mut out = String::new();
    out.push_str("// Patch CommonJS `require('env')` and `require('wasi_snapshot_preview1')` to\n");
    out.push_str("// return shim objects. wasm-pack `--target nodejs` emits bare `require()`\n");
    out.push_str("// calls for these from getrandom/wasi transitives, but they are not real\n");
    out.push_str("// Node modules — the WASM module imports them by name and the host is\n");
    out.push_str("// expected to satisfy them. Patch Module._load BEFORE the wasm bundle is\n");
    out.push_str("// imported by any test file.\n");
    out.push_str("// Note: setupFiles run per-test-worker; vitest imports the test files\n");
    out.push_str("// AFTER setupFiles complete, so this hook installs in time.\n");
    out.push_str("{\n");
    out.push_str("  const _require = createRequire(import.meta.url);\n");
    out.push_str("  const Module = _require('module');\n");
    out.push_str("  // env.system / env.mkstemp come from C-runtime calls embedded in some\n");
    out.push_str("  // WASM-compiled deps (e.g. tesseract-wasm). Tests that don't exercise\n");
    out.push_str("  // those paths only need the imports to be callable for module instantiation.\n");
    out.push_str("  const env = {\n");
    out.push_str("    system: (_cmd: number) => -1,\n");
    out.push_str("    mkstemp: (_template: number) => -1,\n");
    out.push_str("  };\n");
    out.push_str("  // WASI shims. Critical: clock_time_get and random_get must produce realistic\n");
    out.push_str("  // values — returning 0 for all clock calls causes WASM-side timing loops to\n");
    out.push_str("  // spin forever (e.g. getrandom's spin-until-elapsed retry), and zero-filled\n");
    out.push_str("  // random buffers can cause init loops in deps expecting non-zero entropy.\n");
    out.push_str("  const _wasiMemoryView = (): DataView | null => {\n");
    out.push_str("    // Imports are wired before the WASM is instantiated; the bundle stashes\n");
    out.push_str("    // its instance on a runtime-known global once available. We try to grab\n");
    out.push_str("    // it lazily so writes to wasm memory go to the right place.\n");
    out.push_str("    const g = globalThis as unknown as { __alef_wasm_memory__?: WebAssembly.Memory };\n");
    out.push_str("    return g.__alef_wasm_memory__ ? new DataView(g.__alef_wasm_memory__.buffer) : null;\n");
    out.push_str("  };\n");
    out.push_str("  const _cryptoFill = (buf: Uint8Array) => {\n");
    out.push_str("    const c = globalThis.crypto;\n");
    out.push_str("    if (c && typeof c.getRandomValues === 'function') c.getRandomValues(buf);\n");
    out.push_str("    else for (let i = 0; i < buf.length; i++) buf[i] = Math.floor(Math.random() * 256);\n");
    out.push_str("  };\n");
    out.push_str("  const wasi_snapshot_preview1 = {\n");
    out.push_str("    proc_exit: () => {},\n");
    out.push_str("    environ_get: () => 0,\n");
    out.push_str("    environ_sizes_get: (countOut: number, _sizeOut: number) => {\n");
    out.push_str("      const v = _wasiMemoryView();\n");
    out.push_str("      if (v) v.setUint32(countOut, 0, true);\n");
    out.push_str("      return 0;\n");
    out.push_str("    },\n");
    out.push_str("    // WASI fd_write must update `nwritten_ptr` with the total bytes consumed,\n");
    out.push_str("    // otherwise libc-style callers (e.g. tesseract-compiled-to-wasm fputs)\n");
    out.push_str("    // see 0 of N bytes written and retry forever, hanging the host.\n");
    out.push_str("    fd_write: (_fd: number, iovsPtr: number, iovsLen: number, nwrittenPtr: number) => {\n");
    out.push_str("      const v = _wasiMemoryView();\n");
    out.push_str("      if (!v) return 0;\n");
    out.push_str("      let total = 0;\n");
    out.push_str("      for (let i = 0; i < iovsLen; i++) {\n");
    out.push_str("        const off = iovsPtr + i * 8;\n");
    out.push_str("        total += v.getUint32(off + 4, true);\n");
    out.push_str("      }\n");
    out.push_str("      v.setUint32(nwrittenPtr, total, true);\n");
    out.push_str("      return 0;\n");
    out.push_str("    },\n");
    out.push_str("    // Mirror fd_write: callers retry on partial reads. Reporting 0 bytes\n");
    out.push_str("    // read (EOF) is fine; just make sure `nread_ptr` is written.\n");
    out.push_str("    fd_read: (_fd: number, _iovsPtr: number, _iovsLen: number, nreadPtr: number) => {\n");
    out.push_str("      const v = _wasiMemoryView();\n");
    out.push_str("      if (v) v.setUint32(nreadPtr, 0, true);\n");
    out.push_str("      return 0;\n");
    out.push_str("    },\n");
    out.push_str("    fd_seek: () => 0,\n");
    out.push_str("    fd_close: () => 0,\n");
    out.push_str("    fd_prestat_get: () => 8, // EBADF — no preopens.\n");
    out.push_str("    fd_prestat_dir_name: () => 0,\n");
    out.push_str("    fd_fdstat_get: () => 0,\n");
    out.push_str("    fd_fdstat_set_flags: () => 0,\n");
    out.push_str("    path_open: () => 44, // ENOENT.\n");
    out.push_str("    path_create_directory: () => 0,\n");
    out.push_str("    path_remove_directory: () => 0,\n");
    out.push_str("    path_unlink_file: () => 0,\n");
    out.push_str("    path_filestat_get: () => 44, // ENOENT.\n");
    out.push_str("    path_rename: () => 0,\n");
    out.push_str("    clock_time_get: (_clockId: number, _precision: bigint, timeOut: number) => {\n");
    out.push_str("      const ns = BigInt(Date.now()) * 1_000_000n + BigInt(performance.now() | 0) % 1_000_000n;\n");
    out.push_str("      const v = _wasiMemoryView();\n");
    out.push_str("      if (v) v.setBigUint64(timeOut, ns, true);\n");
    out.push_str("      return 0;\n");
    out.push_str("    },\n");
    out.push_str("    clock_res_get: (_clockId: number, resOut: number) => {\n");
    out.push_str("      const v = _wasiMemoryView();\n");
    out.push_str("      if (v) v.setBigUint64(resOut, 1_000n, true);\n");
    out.push_str("      return 0;\n");
    out.push_str("    },\n");
    out.push_str("    random_get: (bufPtr: number, bufLen: number) => {\n");
    out.push_str("      const g = globalThis as unknown as { __alef_wasm_memory__?: WebAssembly.Memory };\n");
    out.push_str("      if (!g.__alef_wasm_memory__) return 0;\n");
    out.push_str("      _cryptoFill(new Uint8Array(g.__alef_wasm_memory__.buffer, bufPtr, bufLen));\n");
    out.push_str("      return 0;\n");
    out.push_str("    },\n");
    out.push_str("    args_get: () => 0,\n");
    out.push_str("    args_sizes_get: (countOut: number, _sizeOut: number) => {\n");
    out.push_str("      const v = _wasiMemoryView();\n");
    out.push_str("      if (v) v.setUint32(countOut, 0, true);\n");
    out.push_str("      return 0;\n");
    out.push_str("    },\n");
    out.push_str("    poll_oneoff: () => 0,\n");
    out.push_str("    sched_yield: () => 0,\n");
    out.push_str("  };\n");
    out.push_str("  const _origResolve = Module._resolveFilename;\n");
    out.push_str("  Module._resolveFilename = function(request: string, parent: unknown, ...rest: unknown[]) {\n");
    out.push_str("    if (request === 'env' || request === 'wasi_snapshot_preview1') return request;\n");
    out.push_str("    return _origResolve.call(this, request, parent, ...rest);\n");
    out.push_str("  };\n");
    out.push_str("  const _origLoad = Module._load;\n");
    out.push_str("  Module._load = function(request: string, parent: unknown, ...rest: unknown[]) {\n");
    out.push_str("    if (request === 'env') return env;\n");
    out.push_str("    if (request === 'wasi_snapshot_preview1') return wasi_snapshot_preview1;\n");
    out.push_str("    return _origLoad.call(this, request, parent, ...rest);\n");
    out.push_str("  };\n");
    out.push_str("  // Capture the WASM linear memory at instantiation time so the WASI shims\n");
    out.push_str("  // can read/write into it. Without this, every shim that needs memory\n");
    out.push_str("  // (fd_write nwritten, clock_time_get, random_get, etc.) silently no-ops\n");
    out.push_str("  // and the host-side C runtime hangs in a retry loop.\n");
    out.push_str("  const _OrigInstance = WebAssembly.Instance;\n");
    out.push_str("  const PatchedInstance = function(this: WebAssembly.Instance, mod: WebAssembly.Module, imports?: WebAssembly.Imports) {\n");
    out.push_str("    const inst = new _OrigInstance(mod, imports);\n");
    out.push_str("    const exportsMem = (inst.exports as Record<string, unknown>).memory;\n");
    out.push_str("    if (exportsMem instanceof WebAssembly.Memory) {\n");
    out.push_str("      (globalThis as unknown as { __alef_wasm_memory__?: WebAssembly.Memory }).__alef_wasm_memory__ = exportsMem;\n");
    out.push_str("    }\n");
    out.push_str("    return inst;\n");
    out.push_str("  } as unknown as typeof WebAssembly.Instance;\n");
    out.push_str("  PatchedInstance.prototype = _OrigInstance.prototype;\n");
    out.push_str(
        "  (WebAssembly as unknown as { Instance: typeof WebAssembly.Instance }).Instance = PatchedInstance;\n",
    );
    out.push_str("}\n\n");
    out.push_str("// Change to the configured test-documents directory so that fixture file paths like\n");
    out.push_str("// \"pdf/fake_memo.pdf\" resolve correctly when vitest runs from e2e/wasm/.\n");
    out.push_str("// setup.ts lives in e2e/wasm/; the fixtures dir lives at the repository root,\n");
    out.push_str("// two directories up: e2e/wasm/ -> e2e/ -> repo root.\n");
    out.push_str("const __filename = fileURLToPath(import.meta.url);\n");
    out.push_str("const __dirname = dirname(__filename);\n");
    let _ = writeln!(
        out,
        "const testDocumentsDir = join(__dirname, '..', '..', '{test_documents_dir}');"
    );
    out.push_str("process.chdir(testDocumentsDir);\n");
    out
}

fn render_tsconfig() -> String {
    crate::e2e::template_env::render("wasm/tsconfig.jinja", minijinja::context! {})
}
// The historical `inject_wasm_init` post-processor rewrote test imports to a
// `<pkg>/dist-node` subpath. It was removed because the alef-managed
// `wasm-pack build --target nodejs` artifact is a flat self-initializing CJS
// module — its `package.json` already sets `"main"` to the JS entry, so the
// emitted `import … from "<pkg>"` resolves directly.

#[cfg(test)]
mod tests;
