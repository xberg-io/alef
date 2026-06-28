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

    // The wasm crate's Cargo.toml is managed by alef-backend-wasm's generate_bindings
    // (emitted alongside lib.rs so it is always regenerated). The scaffold only owns
    // the package.json.

    let mut files = vec![];

    let wasm_pkg_name = config.wasm_package_name();

    // wasm-pack converts hyphens to underscores in the generated filenames
    // (Rust convention), so `demo-markup` becomes `demo_markup`.
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
  "files": [
    "pkg",
    "*.wasm",
    "*.d.ts",
    "README.md"
  ],
  "main": "pkg/nodejs/{core_crate_file}_wasm.js",
  "module": "pkg/web/{core_crate_file}_wasm.js",
  "types": "pkg/nodejs/{core_crate_file}_wasm.d.ts",
  "engines": {{
    "node": "{node_engine}"
  }},
  "scripts": {{
    "build": "wasm-pack build --target nodejs --out-dir pkg/nodejs",
    "build:ci": "wasm-pack build --release --target nodejs --out-dir pkg/nodejs",
    "build:wasm:web": "wasm-pack build --release --target web --out-dir pkg/web",
    "build:wasm:bundler": "wasm-pack build --release --target bundler --out-dir pkg/bundler",
    "build:wasm:nodejs": "wasm-pack build --release --target nodejs --out-dir pkg/nodejs",
    "build:wasm:deno": "wasm-pack build --release --target deno --out-dir pkg/deno",
    "build:all": "npm run build:wasm:web && npm run build:wasm:bundler && npm run build:wasm:nodejs && npm run build:wasm:deno && find pkg -name .gitignore -delete",
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
        core_crate_file = core_crate_file,
        node_engine = tv::npm::NODE_ENGINE,
    );

    files.push(GeneratedFile {
        path: PathBuf::from(format!("crates/{}-wasm/package.json", core_crate_dir)),
        content: pkg_json,
        generated_header: false,
    });

    Ok(files)
}
