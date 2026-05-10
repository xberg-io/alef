//! WebAssembly e2e test generator using vitest.
//!
//! Reuses the TypeScript test renderer for both HTTP and non-HTTP fixtures,
//! configured with the `@kreuzberg/wasm` (or equivalent) package as the import
//! path and `wasm` as the language key for skip/override resolution. Adds
//! wasm-specific scaffolding: vite-plugin-wasm + top-level-await for vitest,
//! a `setup.ts` chdir to `test_documents/` so file_path fixtures resolve, and
//! a `globalSetup.ts` that spawns the mock-server for HTTP fixtures.

use crate::config::E2eConfig;
use crate::escape::sanitize_filename;
use crate::field_access::FieldResolver;
use crate::fixture::{Fixture, FixtureGroup};
use alef_core::backend::GeneratedFile;
use alef_core::config::ResolvedCrateConfig;
use alef_core::hash::{self, CommentStyle};
use alef_core::template_versions as tv;
use anyhow::Result;
use std::path::PathBuf;

use super::E2eCodegen;

/// WebAssembly e2e code generator.
pub struct WasmCodegen;

impl E2eCodegen for WasmCodegen {
    fn generate(
        &self,
        groups: &[FixtureGroup],
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        type_defs: &[alef_core::ir::TypeDef],
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
        let pkg_path = wasm_pkg
            .as_ref()
            .and_then(|p| p.path.as_ref())
            .cloned()
            .unwrap_or_else(|| config.wasm_crate_path());
        let pkg_name = wasm_pkg
            .as_ref()
            .and_then(|p| p.name.as_ref())
            .cloned()
            .unwrap_or_else(|| {
                // Default: derive from WASM crate name (config.name + "-wasm")
                // wasm-pack transforms the crate name to the package name by replacing
                // dashes with the crate separator in Cargo (e.g., kreuzberg-wasm -> kreuzberg_wasm).
                // However, the published npm package might use the module name, which is typically
                // the crate name without "-wasm". Fall back to the module path.
                module_path.clone()
            });
        let pkg_version = wasm_pkg
            .as_ref()
            .and_then(|p| p.version.as_ref())
            .cloned()
            .or_else(|| config.resolved_version())
            .unwrap_or_else(|| "0.1.0".to_string());

        // Determine which auxiliary scaffolding files we need based on the active
        // fixture set. Doing this once up front lets us emit a self-contained vitest
        // config that wires only the setup files we'll actually generate.
        let active_per_group: Vec<Vec<&Fixture>> = groups
            .iter()
            .map(|group| {
                group
                    .fixtures
                    .iter()
                    .filter(|f| super::should_include_fixture(f, lang, e2e_config))
                    // Honor per-call `skip_languages`: when the resolved call's
                    // `skip_languages` contains `wasm`, the wasm binding doesn't
                    // export that function and any test file referencing it
                    // would fail TS resolution. Drop the fixture entirely.
                    .filter(|f| {
                        let cc = e2e_config.resolve_call_for_fixture(f.call.as_deref(), &f.input);
                        !cc.skip_languages.iter().any(|l| l == lang)
                    })
                    .filter(|f| {
                        // Node fetch (undici) rejects pre-set Content-Length that
                        // doesn't match the real body length — skip fixtures that
                        // intentionally send a mismatched header.
                        f.http.as_ref().is_none_or(|h| {
                            !h.request
                                .headers
                                .iter()
                                .any(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                        })
                    })
                    .filter(|f| {
                        // Node fetch only supports a fixed set of HTTP methods;
                        // TRACE and CONNECT throw before reaching the server.
                        f.http.as_ref().is_none_or(|h| {
                            let m = h.request.method.to_ascii_uppercase();
                            m != "TRACE" && m != "CONNECT"
                        })
                    })
                    .collect()
            })
            .collect();

        let any_fixtures = active_per_group.iter().flat_map(|g| g.iter());
        let has_http_fixtures = any_fixtures.clone().any(|f| f.is_http_test());
        // file_path / bytes args are read off disk by the generated code at runtime;
        // we add a setup.ts chdir to test_documents so relative paths resolve.
        let has_file_fixtures = active_per_group.iter().flatten().any(|f| {
            let cc = e2e_config.resolve_call_for_fixture(f.call.as_deref(), &f.input);
            cc.args
                .iter()
                .any(|a| a.arg_type == "file_path" || a.arg_type == "bytes")
        });

        // Generate package.json — adds vite-plugin-wasm + top-level-await on top
        // of the standard vitest dev deps so that `import init, { … } from
        // '@kreuzberg/wasm'` resolves and instantiates the wasm module before tests
        // run.
        files.push(GeneratedFile {
            path: output_base.join("package.json"),
            content: render_package_json(&pkg_name, &pkg_path, &pkg_version, e2e_config.dep_mode),
            generated_header: false,
        });

        // Generate vitest.config.ts — needs vite-plugin-wasm + topLevelAwait, plus
        // optional globalSetup (for HTTP fixtures and any function-call test that
        // hits the mock server via MOCK_SERVER_URL) and setupFiles (for chdir).
        // Function-call e2e tests construct request URLs via
        // `${process.env.MOCK_SERVER_URL}/fixtures/<id>`, so the mock server must
        // be running and the env var set even when no raw HTTP fixtures exist.
        let needs_global_setup = has_http_fixtures;
        files.push(GeneratedFile {
            path: output_base.join("vitest.config.ts"),
            content: render_vitest_config(needs_global_setup, has_file_fixtures),
            generated_header: true,
        });

        // Generate globalSetup.ts when any fixture requires the mock server —
        // either an HTTP fixture (the original consumer) or any function-call
        // fixture that interpolates `${process.env.MOCK_SERVER_URL}` into a
        // base URL. It spawns the rust mock-server binary.
        if needs_global_setup {
            files.push(GeneratedFile {
                path: output_base.join("globalSetup.ts"),
                content: render_global_setup(),
                generated_header: true,
            });
        }

        // Generate setup.ts when any active fixture takes a file_path / bytes arg.
        // This chdir's to test_documents/ so relative fixture paths resolve.
        if has_file_fixtures {
            files.push(GeneratedFile {
                path: output_base.join("setup.ts"),
                content: render_file_setup(),
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

        // Resolve options_type from override (e.g. `WasmExtractionConfig`).
        let options_type = overrides.and_then(|o| o.options_type.clone());
        let field_resolver = FieldResolver::new(
            &e2e_config.fields,
            &e2e_config.fields_optional,
            &e2e_config.result_fields,
            &e2e_config.fields_array,
            &std::collections::HashSet::new(),
        );

        // Generate test files per category. We delegate the per-fixture rendering
        // to the typescript codegen (`render_test_file`), which already handles
        // both HTTP and function-call fixtures correctly. Passing `lang = "wasm"`
        // routes per-fixture override resolution and skip checks through the wasm
        // language key. We then inject Node.js WASM initialization code to load
        // the WASM binary from the pkg directory using fs.readFileSync.
        for (group, active) in groups.iter().zip(active_per_group.iter()) {
            if active.is_empty() {
                continue;
            }
            let filename = format!("{}.test.ts", sanitize_filename(&group.category));
            let content = super::typescript::render_test_file(
                lang,
                &group.category,
                active,
                &module_path,
                &pkg_name,
                &function_name,
                &e2e_config.call.args,
                options_type.as_deref(),
                &field_resolver,
                client_factory,
                e2e_config,
                type_defs,
            );

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

        Ok(files)
    }

    fn language_name(&self) -> &'static str {
        "wasm"
    }
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
    pkg_version: &str,
    dep_mode: crate::config::DependencyMode,
) -> String {
    let dep_value = match dep_mode {
        crate::config::DependencyMode::Registry => pkg_version.to_string(),
        crate::config::DependencyMode::Local => format!("file:{pkg_path}"),
    };
    crate::template_env::render(
        "wasm/package.json.jinja",
        minijinja::context! {
            pkg_name => pkg_name,
            dep_value => dep_value,
            rollup => tv::npm::ROLLUP,
            vite_plugin_wasm => tv::npm::VITE_PLUGIN_WASM,
            vitest => tv::npm::VITEST,
        },
    )
}

fn render_vitest_config(with_global_setup: bool, with_file_setup: bool) -> String {
    let header = hash::header(CommentStyle::DoubleSlash);
    crate::template_env::render(
        "wasm/vitest.config.ts.jinja",
        minijinja::context! {
            header => header,
            with_global_setup => with_global_setup,
            with_file_setup => with_file_setup,
        },
    )
}

fn render_file_setup() -> String {
    let header = hash::header(CommentStyle::DoubleSlash);
    header
        + r#"import { fileURLToPath } from 'url';
import { dirname, join } from 'path';

// Change to the test_documents directory so that fixture file paths like
// "pdf/fake_memo.pdf" resolve correctly when vitest runs from e2e/wasm/.
// setup.ts lives in e2e/wasm/; test_documents lives at the repository root,
// two directories up: e2e/wasm/ -> e2e/ -> repo root -> test_documents/.
const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const testDocumentsDir = join(__dirname, '..', '..', 'test_documents');
process.chdir(testDocumentsDir);
"#
}

fn render_global_setup() -> String {
    let header = hash::header(CommentStyle::DoubleSlash);
    crate::template_env::render(
        "wasm/globalSetup.ts.jinja",
        minijinja::context! {
            header => header,
        },
    )
}

fn render_tsconfig() -> String {
    crate::template_env::render("wasm/tsconfig.jinja", minijinja::context! {})
}

// The historical `inject_wasm_init` post-processor rewrote test imports to a
// `<pkg>/dist-node` subpath. It was removed because the alef-managed
// `wasm-pack build --target nodejs` artifact is a flat self-initializing CJS
// module — its `package.json` already sets `"main"` to the JS entry, so the
// emitted `import … from "<pkg>"` resolves directly.
