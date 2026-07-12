use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::ApiSurface;
use crate::core::template_versions as tv;
use crate::scaffold::scaffold_meta;
use std::path::PathBuf;

pub(crate) fn scaffold_wasm(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let version = &api.version;
    let core_crate_dir = config.core_crate_dir();

    let mut files = vec![];

    let wasm_pkg_name = config.wasm_package_name();

    let core_crate_file = core_crate_dir.replace('-', "_");
    let repository_block = meta
        .configured_repository
        .as_deref()
        .map(|repository| {
            format!(
                r#",
  "repository": {{
    "type": "git",
    "url": "{repository}",
    "directory": "crates/{core_crate_dir}-wasm"
  }}"#
            )
        })
        .unwrap_or_default();
    let license_block = meta
        .license
        .as_deref()
        .map(|license| format!(",\n  \"license\": \"{license}\""))
        .unwrap_or_default();

    // wasm-pack build targets that ship in this package. Each target embeds a
    // full copy of the wasm binary, so a single-target set keeps the published
    // package small. Derives `files`, entry points, and the build scripts.
    let targets = config.wasm_targets();
    const VALID_TARGETS: &[&str] = &["web", "bundler", "nodejs", "deno"];
    if targets.is_empty() {
        anyhow::bail!("[crates.wasm].targets must list at least one wasm-pack target (web, bundler, nodejs, deno)");
    }
    for target in &targets {
        if !VALID_TARGETS.contains(&target.as_str()) {
            anyhow::bail!("[crates.wasm].targets: unknown target '{target}' (valid: web, bundler, nodejs, deno)");
        }
    }
    let has = |t: &str| targets.iter().any(|x| x == t);
    // `main`/`types` prefer the CommonJS-friendly nodejs build; `module` prefers
    // the browser ES module. When a preferred target isn't built, fall back to
    // the first configured target.
    let node_target = if has("nodejs") { "nodejs" } else { targets[0].as_str() };
    let web_target = if has("web") { "web" } else { targets[0].as_str() };

    // A single-target package publishes just that target's dir; a multi-target
    // package keeps the broad glob for backward compatibility.
    let files_block = if targets.len() == 1 {
        format!("[\n    \"pkg/{}\",\n    \"README.md\"\n  ]", targets[0])
    } else {
        "[\n    \"pkg\",\n    \"*.wasm\",\n    \"*.d.ts\",\n    \"README.md\"\n  ]".to_string()
    };

    let per_target_scripts: String = targets
        .iter()
        .map(|t| format!("    \"build:wasm:{t}\": \"wasm-pack build --release --target {t} --out-dir pkg/{t}\",\n"))
        .collect();
    let build_all = targets
        .iter()
        .map(|t| format!("npm run build:wasm:{t}"))
        .collect::<Vec<_>>()
        .join(" && ");

    let pkg_json = format!(
        r#"{{
  "name": "{wasm_pkg_name}",
  "version": "{version}",
  "private": false,
  "description": "{description}"{license_block}{repository_block},
  "publishConfig": {{
    "access": "public"
  }},
  "type": "module",
  "files": {files_block},
  "main": "pkg/{node_target}/{core_crate_file}_wasm.js",
  "module": "pkg/{web_target}/{core_crate_file}_wasm.js",
  "types": "pkg/{node_target}/{core_crate_file}_wasm.d.ts",
  "engines": {{
    "node": "{node_engine}"
  }},
  "scripts": {{
    "build": "wasm-pack build --target {node_target} --out-dir pkg/{node_target}",
    "build:ci": "wasm-pack build --release --target {node_target} --out-dir pkg/{node_target}",
{per_target_scripts}    "build:all": "{build_all} && find pkg -name .gitignore -delete",
    "test": "vitest run",
    "test:watch": "vitest watch",
    "test:coverage": "vitest run --coverage",
    "clean": "rm -rf pkg dist"
  }}
}}
"#,
        wasm_pkg_name = wasm_pkg_name,
        version = version,
        description = meta.description,
        license_block = license_block,
        repository_block = repository_block,
        files_block = files_block,
        node_target = node_target,
        web_target = web_target,
        core_crate_file = core_crate_file,
        node_engine = tv::npm::NODE_ENGINE,
        per_target_scripts = per_target_scripts,
        build_all = build_all,
    );

    files.push(GeneratedFile {
        path: PathBuf::from(format!("crates/{}-wasm/package.json", core_crate_dir)),
        content: pkg_json,
        generated_header: false,
    });

    Ok(files)
}
