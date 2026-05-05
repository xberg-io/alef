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
        // For projects with a core library name different from the package name,
        // try both {config.name}-wasm and ts-pack-core-wasm (for tree-sitter-language-pack).
        let wasm_pkg = e2e_config.resolve_package("wasm");
        let pkg_path = wasm_pkg
            .as_ref()
            .and_then(|p| p.path.as_ref())
            .cloned()
            .unwrap_or_else(|| {
                let default_name = format!("../../crates/{}-wasm/pkg", config.name);
                // Special case: tree-sitter-language-pack uses ts-pack-core-wasm
                if config.name == "tree-sitter-language-pack" {
                    "../../crates/ts-pack-core-wasm/pkg".to_string()
                } else {
                    default_name
                }
            });
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
                    .filter(|f| f.skip.as_ref().is_none_or(|s| !s.should_skip(lang)))
                    // Honor per-call `skip_languages`: when the resolved call's
                    // `skip_languages` contains `wasm`, the wasm binding doesn't
                    // export that function and any test file referencing it
                    // would fail TS resolution. Drop the fixture entirely.
                    .filter(|f| {
                        let cc = e2e_config.resolve_call(f.call.as_deref());
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
        let has_non_http_fixtures = any_fixtures
            .clone()
            .any(|f| !f.is_http_test() && !f.assertions.is_empty());
        // file_path / bytes args are read off disk by the generated code at runtime;
        // we add a setup.ts chdir to test_documents so relative paths resolve.
        let has_file_fixtures = active_per_group.iter().flatten().any(|f| {
            let cc = e2e_config.resolve_call(f.call.as_deref());
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
        // optional globalSetup (for HTTP fixtures) and setupFiles (for chdir).
        files.push(GeneratedFile {
            path: output_base.join("vitest.config.ts"),
            content: render_vitest_config(has_http_fixtures, has_file_fixtures),
            generated_header: true,
        });

        // Generate globalSetup.ts only when at least one HTTP fixture is in scope —
        // it spawns the rust mock-server.
        if has_http_fixtures {
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

        // Suppress the unused-variable warning when no non-HTTP fixtures exist.
        let _ = has_non_http_fixtures;

        // Resolve options_type from override (e.g. `WasmExtractionConfig`).
        let options_type = overrides.and_then(|o| o.options_type.clone());
        let field_resolver = FieldResolver::new(
            &e2e_config.fields,
            &e2e_config.fields_optional,
            &e2e_config.result_fields,
            &e2e_config.fields_array,
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
            let mut content = super::typescript::render_test_file(
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
            );

            // Inject WASM initialization code for Node.js environments.
            // Pass the WASM crate name (e.g., "html-to-markdown-wasm") instead of the core crate name.
            let wasm_crate_name = format!("{}-wasm", config.name);
            content = inject_wasm_init(&content, &pkg_name, &wasm_crate_name);

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
    format!(
        r#"{{
  "name": "{pkg_name}-e2e-wasm",
  "version": "0.1.0",
  "private": true,
  "type": "module",
  "scripts": {{
    "test": "vitest run"
  }},
  "devDependencies": {{
    "{pkg_name}": "{dep_value}",
    "rollup": "{rollup}",
    "vite-plugin-wasm": "{vite_plugin_wasm}",
    "vitest": "{vitest}"
  }}
}}
"#,
        rollup = tv::npm::ROLLUP,
        vite_plugin_wasm = tv::npm::VITE_PLUGIN_WASM,
        vitest = tv::npm::VITEST,
    )
}

fn render_vitest_config(with_global_setup: bool, with_file_setup: bool) -> String {
    let header = hash::header(CommentStyle::DoubleSlash);
    let setup_files_line = if with_file_setup {
        "    setupFiles: ['./setup.ts'],\n"
    } else {
        ""
    };
    let global_setup_line = if with_global_setup {
        "    globalSetup: './globalSetup.ts',\n"
    } else {
        ""
    };
    format!(
        r#"{header}import {{ defineConfig }} from 'vitest/config';
import wasm from 'vite-plugin-wasm';

export default defineConfig({{
  plugins: [wasm()],
  test: {{
    include: ['tests/**/*.test.ts'],
{global_setup_line}{setup_files_line}  }},
}});
"#
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
    format!(
        r#"{header}import {{ spawn }} from 'child_process';
import {{ resolve }} from 'path';

let serverProcess: any;

export async function setup() {{
  // Mock server binary must be pre-built (e.g. by CI or `cargo build --manifest-path e2e/rust/Cargo.toml --bin mock-server --release`)
  serverProcess = spawn(
    resolve(__dirname, '../rust/target/release/mock-server'),
    [resolve(__dirname, '../../fixtures')],
    {{ stdio: ['pipe', 'pipe', 'inherit'] }}
  );

  const url = await new Promise<string>((resolve, reject) => {{
    serverProcess.stdout.on('data', (data: Buffer) => {{
      const match = data.toString().match(/MOCK_SERVER_URL=(.*)/);
      if (match) resolve(match[1].trim());
    }});
    setTimeout(() => reject(new Error('Mock server startup timeout')), 30000);
  }});

  process.env.MOCK_SERVER_URL = url;
}}

export async function teardown() {{
  if (serverProcess) {{
    serverProcess.stdin.end();
    serverProcess.kill();
  }}
}}
"#
    )
}

fn render_tsconfig() -> String {
    r#"{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ESNext",
    "moduleResolution": "bundler",
    "strict": true,
    "strictNullChecks": false,
    "esModuleInterop": true,
    "skipLibCheck": true
  },
  "include": ["tests/**/*.ts", "vitest.config.ts"]
}
"#
    .to_string()
}

/// Inject WASM initialization code for Node.js environments.
///
/// Injects top-level await for the async init() function from wasm-pack.
/// This allows the WASM module to be initialized before tests run.
///
/// # Arguments
/// * `content` — the generated TypeScript test file content
/// * `pkg_name` — the npm package name (e.g., "kreuzberg" or "@org/kreuzberg")
/// * `_crate_name` — the Rust crate name (unused in async init pattern)
fn inject_wasm_init(content: &str, pkg_name: &str, _crate_name: &str) -> String {
    // The TypeScript renderer generates single-quoted imports; match both styles for robustness.
    let from_marker_sq = format!("}} from '{pkg_name}';");
    let from_marker_dq = format!("}} from \"{pkg_name}\";");
    let from_marker = if content.contains(&from_marker_sq) {
        from_marker_sq
    } else {
        from_marker_dq
    };

    // Find the closing `} from "pkg_name";` marker, then search backward for the matching `import {`
    // to avoid accidentally patching an earlier import statement (e.g. `import { ... } from "vitest"`).
    if let Some(from_pos) = content.find(&from_marker) {
        let full_from_pos = from_pos + from_marker.len();
        // Search backward from from_pos to find the last `import {` or `import init, {` before it.
        let before_from = &content[..from_pos];
        if let Some(import_pos) = before_from
            .rfind("import {")
            .or_else(|| before_from.rfind("import init, {"))
        {
            let import_section = &content[import_pos..full_from_pos];

            // Already patched (contains `import init`) — nothing to do.
            if import_section.contains("import init,") {
                return content.to_string();
            }

            // wasm-pack exports `init` as the default export, not a named export.
            // Transform `import { ... }` to `import init, { ... }` so the default
            // export is bound and `await init()` works at the top level.
            let new_import = import_section.replacen("import {", "import init, {", 1);

            // Node.js fetch does not support file:// URLs, so we cannot call init() without
            // arguments (which internally calls fetch on the .wasm file URL). Instead, read the
            // binary via readFileSync and pass the buffer directly to init(), bypassing fetch.
            // We resolve the .wasm path from the installed package directory by replacing the .js
            // main entry extension. Dynamic imports avoid adding new static import statements that
            // would require import-order adjustments.
            let init_code = format!(
                concat!(
                    "await init(\n",
                    "  (await import(\"node:fs\")).readFileSync(\n",
                    "    (await import(\"node:module\"))\n",
                    "      .createRequire(import.meta.url)\n",
                    "      .resolve(\"{pkg_name}\")\n",
                    "      .replace(/\\.js$/, \"_bg.wasm\"),\n",
                    "  ),\n",
                    ");\n",
                ),
                pkg_name = pkg_name,
            );

            return content[..import_pos].to_string() + &new_import + "\n" + &init_code + &content[full_from_pos..];
        }
    }

    content.to_string()
}
