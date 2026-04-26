use crate::scaffold_meta;
use alef_core::backend::GeneratedFile;
use alef_core::config::AlefConfig;
use alef_core::ir::ApiSurface;
use std::path::PathBuf;

pub(crate) fn scaffold_wasm(api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let version = &api.version;
    let core_crate_dir = config.core_crate_dir();

    // The wasm crate's Cargo.toml is managed by alef-backend-wasm's generate_bindings
    // (emitted alongside lib.rs so it is always regenerated). The scaffold only owns
    // the package.json.

    let mut files = vec![];

    // Generate package.json for npm publishing.
    // Uses the node package name with -wasm suffix for the npm scope.
    let node_pkg = config.node_package_name();
    let wasm_pkg_name = format!("{node_pkg}-wasm");
    let pkg_json = format!(
        r#"{{
  "name": "{wasm_pkg_name}",
  "version": "{version}",
  "private": false,
  "description": "{description}",
  "license": "{license}",
  "repository": {{
    "type": "git",
    "url": "{repository}",
    "directory": "crates/{core_crate_dir}-wasm"
  }},
  "type": "module",
  "files": [
    "pkg",
    "*.wasm",
    "*.d.ts",
    "README.md"
  ],
  "main": "pkg/nodejs/{core_crate_dir}_wasm.js",
  "module": "pkg/web/{core_crate_dir}_wasm.js",
  "types": "pkg/nodejs/{core_crate_dir}_wasm.d.ts",
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
        license = meta.license,
        repository = meta.repository,
        core_crate_dir = core_crate_dir,
    );

    files.push(GeneratedFile {
        path: PathBuf::from(format!("crates/{}-wasm/package.json", core_crate_dir)),
        content: pkg_json,
        generated_header: false,
    });

    Ok(files)
}
