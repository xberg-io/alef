use crate::{cargo_package_header, core_dep_features, detect_workspace_inheritance, render_extra_deps, scaffold_meta};
use alef_core::backend::GeneratedFile;
use alef_core::config::{AdapterPattern, Language, ResolvedCrateConfig};
use alef_core::ir::{ApiSurface, TypeRef};
use alef_core::template_versions as tv;
use std::path::PathBuf;

/// Check if a TypeRef or any of its nested types is Json
fn type_ref_contains_json(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Json => true,
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => type_ref_contains_json(inner),
        TypeRef::Map(key, val) => type_ref_contains_json(key) || type_ref_contains_json(val),
        _ => false,
    }
}

/// Check if the API surface has any Json fields
fn api_has_json_fields(api: &ApiSurface) -> bool {
    // Check struct fields
    for type_def in &api.types {
        for field in &type_def.fields {
            if type_ref_contains_json(&field.ty) {
                return true;
            }
        }
        // Check method parameters and return types
        for method in &type_def.methods {
            if type_ref_contains_json(&method.return_type) {
                return true;
            }
            for param in &method.params {
                if type_ref_contains_json(&param.ty) {
                    return true;
                }
            }
        }
    }

    // Check function parameters and return types
    for func in &api.functions {
        if type_ref_contains_json(&func.return_type) {
            return true;
        }
        for param in &func.params {
            if type_ref_contains_json(&param.ty) {
                return true;
            }
        }
    }

    // Check enum fields
    for enum_def in &api.enums {
        for variant in &enum_def.variants {
            for field in &variant.fields {
                if type_ref_contains_json(&field.ty) {
                    return true;
                }
            }
        }
    }

    false
}

pub(crate) fn scaffold_node_cargo(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let version = &api.version;
    let core_crate_dir = config.core_crate_dir();
    let ws = detect_workspace_inheritance(config.workspace_root.as_deref());
    let pkg_header = cargo_package_header(
        &format!("{core_crate_dir}-node"),
        version,
        "2024",
        &meta.license,
        &meta.description,
        &meta.keywords,
        &ws,
    );

    let extra_deps = render_extra_deps(config, Language::Node);

    let has_trait_bridges = !config.trait_bridges.is_empty();
    let has_streaming = config
        .adapters
        .iter()
        .any(|a| matches!(a.pattern, AdapterPattern::Streaming));
    let mut all_deps = extra_deps;
    if has_trait_bridges && !all_deps.contains("async-trait") {
        if !all_deps.is_empty() {
            all_deps.push('\n');
        }
        all_deps.push_str("async-trait = \"0.1\"");
    }
    if has_streaming && !all_deps.contains("futures-util = ") && !all_deps.contains("futures-util =\"") {
        if !all_deps.is_empty() {
            all_deps.push('\n');
        }
        all_deps.push_str("futures-util = \"0.3\"");
    }

    let extra_deps_section = if all_deps.is_empty() {
        String::new()
    } else {
        format!("\n{all_deps}")
    };

    let mut napi_features = vec!["async"];
    if api_has_json_fields(api) {
        napi_features.push("serde-json");
    }
    let napi_features_str = napi_features
        .iter()
        .map(|f| format!("\"{}\"", f))
        .collect::<Vec<_>>()
        .join(", ");

    let content = format!(
        r#"{pkg_header}

[lib]
crate-type = ["cdylib"]

[dependencies]
{crate_name} = {{ path = "../{core_crate_dir}"{features} }}
napi = {{ version = "{napi}", features = [{napi_features}] }}
napi-derive = "{napi_derive}"
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"{extra_deps_section}

# `serde_json` is emitted unconditionally above so the manifest is stable
# across regens, but for umbrella crates with no JSON-marshalled return types
# it is genuinely unused. List it here so `cargo machete` doesn't flag the
# no-json case as a real finding.
[package.metadata.cargo-machete]
ignored = ["serde_json"]

[build-dependencies]
napi-build = "{napi_build}"

"#,
        pkg_header = pkg_header,
        crate_name = &config.name,
        core_crate_dir = core_crate_dir,
        features = core_dep_features(config, Language::Node),
        napi = tv::cargo::NAPI,
        napi_features = napi_features_str,
        napi_derive = tv::cargo::NAPI_DERIVE,
        napi_build = tv::cargo::NAPI_BUILD,
        extra_deps_section = extra_deps_section,
    );

    Ok(vec![GeneratedFile {
        path: PathBuf::from(format!("crates/{}-node/Cargo.toml", core_crate_dir)),
        content,
        generated_header: true,
    }])
}

fn generate_napi_platform_dispatch_index(binary_name: &str) -> String {
    format!(
        r#""use strict";

const {{ platform, arch }} = process;
const isWindows = platform === "win32";
const isMusl = () => {{
  if (
    typeof process.report === "object" &&
    typeof process.report.getReport === "function"
  ) {{
    const report = process.report.getReport();
    if (
      report &&
      report.header &&
      typeof report.header.glibcVersion === "string"
    ) {{
      return false;
    }}
    if (report && report.header && report.header.glibcVersion === undefined) {{
      return true;
    }}
  }}
  try {{
    require("fs").statSync("/lib64/ld-musl-x86_64.so.1");
    return true;
  }} catch {{
    return false;
  }}
}};

let nativeBinding = null;
const loadErrors = [];

function requireOptionalDependency(name) {{
  try {{
    return require(name);
  }} catch (e) {{
    loadErrors.push(`Optional dependency ${{name}}: ${{e.message}}`);
    return null;
  }}
}}

const tryLoadBinding = () => {{
  const targets = [
    ["linux", "x64", "gnu", "./{}.linux-x64-gnu.node", "{}-linux-x64-gnu"],
    ["linux", "x64", "musl", "./{}.linux-x64-musl.node", "{}-linux-x64-musl"],
    ["linux", "arm64", "gnu", "./{}.linux-arm64-gnu.node", "{}-linux-arm64-gnu"],
    ["linux", "arm64", "musl", "./{}.linux-arm64-musl.node", "{}-linux-arm64-musl"],
    ["darwin", "x64", null, "./{}.darwin-x64.node", "{}-darwin-x64"],
    ["darwin", "arm64", null, "./{}.darwin-arm64.node", "{}-darwin-arm64"],
    ["win32", "x64", null, "./{}.win32-x64-msvc.node", "{}-win32-x64-msvc"],
    ["win32", "arm64", null, "./{}.win32-arm64-msvc.node", "{}-win32-arm64-msvc"],
  ];

  for (const [plat, a, abi, localPath, optionalDep] of targets) {{
    if (platform !== plat || arch !== a) {{
      continue;
    }}

    if (plat === "linux" && abi) {{
      const isCurMusl = isMusl();
      if ((abi === "musl") !== isCurMusl) {{
        continue;
      }}
    }}

    try {{
      nativeBinding = require(localPath);
      if (nativeBinding) {{
        return;
      }}
    }} catch (e) {{
      loadErrors.push(e.message);
    }}

    try {{
      const optBinding = requireOptionalDependency(optionalDep);
      if (optBinding) {{
        nativeBinding = optBinding;
        return;
      }}
    }} catch (e) {{
      loadErrors.push(e.message);
    }}
  }}
}};

tryLoadBinding();

if (!nativeBinding) {{
  throw new Error(
    `Failed to load native binding for ${{platform}}-${{arch}}. Errors: ${{loadErrors.join(", ")}}`
  );
}}

module.exports = nativeBinding;
"#,
        binary_name,
        binary_name,
        binary_name,
        binary_name,
        binary_name,
        binary_name,
        binary_name,
        binary_name,
        binary_name,
        binary_name,
        binary_name,
        binary_name,
        binary_name,
        binary_name,
        binary_name,
        binary_name,
    )
}

pub(crate) fn scaffold_node(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let package_name = config.node_package_name();
    let name = &config.name;
    let version = &api.version;
    let pkg_dir = config.package_dir(Language::Node);
    let crate_dir = config.core_crate_dir();

    let keywords_json = if meta.keywords.is_empty() {
        String::new()
    } else {
        let entries: Vec<String> = meta.keywords.iter().map(|k| format!("\"{}\"", k)).collect();
        format!(",\n  \"keywords\": [{}]", entries.join(", "))
    };

    let homepage_json = if meta.homepage.is_empty() {
        String::new()
    } else {
        format!(",\n  \"homepage\": \"{}\"", meta.homepage)
    };

    let authors_json = if meta.authors.is_empty() {
        String::new()
    } else {
        format!(",\n  \"author\": \"{}\"", meta.authors.join(", "))
    };

    let content = format!(
        r#"{{
  "name": "{package_name}",
  "version": "{version}",
  "description": "{description}",
  "license": "{license}",
  "main": "index.js",
  "types": "index.d.ts",
  "repository": "{repository}"{homepage}{authors}{keywords},
  "files": [
    "index.js",
    "index.d.ts",
    "**/*.node"
  ],
  "scripts": {{
    "build": "napi build --release",
    "build:debug": "napi build",
    "build:ts": "echo 'No TypeScript wrapper to build'",
    "test": "node -e \"console.log('Add test command')\"",
    "format": "oxfmt",
    "format:check": "oxfmt --check",
    "lint": "oxlint",
    "lint:fix": "oxlint --fix"
  }},
  "napi": {{
    "name": "{name}",
    "triples": [
      "x86_64-unknown-linux-gnu",
      "x86_64-apple-darwin",
      "aarch64-apple-darwin",
      "x86_64-pc-windows-msvc"
    ]
  }},
  "devDependencies": {{
    "@napi-rs/cli": "{napi_rs_cli_devdeps}",
    "oxfmt": "latest",
    "oxlint": "latest",
    "typescript": "{typescript}"
  }}
}}
"#,
        package_name = package_name,
        version = version,
        description = meta.description,
        license = meta.license,
        repository = meta.repository,
        homepage = homepage_json,
        authors = authors_json,
        keywords = keywords_json,
        name = name,
        napi_rs_cli_devdeps = tv::npm::NAPI_RS_CLI_DEVDEPS,
        typescript = tv::npm::TYPESCRIPT,
    );

    // Crate-level package.json required by `napi build`
    let crate_pkg = format!(
        r#"{{
  "name": "{package_name}",
  "version": "{version}",
  "description": "{description}",
  "license": "{license}",
  "main": "index.js",
  "types": "index.d.ts",
  "files": ["index.js", "index.d.ts", "*.node"],
  "napi": {{
    "binaryName": "{crate_dir}-node",
    "targets": [
      "x86_64-unknown-linux-gnu",
      "aarch64-unknown-linux-gnu",
      "x86_64-apple-darwin",
      "aarch64-apple-darwin",
      "x86_64-pc-windows-msvc"
    ]
  }},
  "scripts": {{
    "build": "napi build --platform --release",
    "artifacts": "napi artifacts",
    "prepublishOnly": "napi prepublish -t npm --skip-optional-publish"
  }},
  "engines": {{ "node": ">= 18" }},
  "devDependencies": {{ "@napi-rs/cli": "{napi_rs_cli_crate}" }}
}}
"#,
        package_name = package_name,
        version = version,
        description = meta.description,
        license = meta.license,
        crate_dir = crate_dir,
        napi_rs_cli_crate = tv::npm::NAPI_RS_CLI_CRATE,
    );

    let dts_content = format!(
        r#"export * from "../../crates/{crate_dir}-node/index";
"#,
        crate_dir = crate_dir,
    );

    let js_content = format!(
        r#"module.exports = require("../../crates/{crate_dir}-node/index.js");
"#,
        crate_dir = crate_dir,
    );

    let crate_index_js = generate_napi_platform_dispatch_index(&format!("{}-node", crate_dir));

    Ok(vec![
        GeneratedFile {
            path: PathBuf::from(format!("{pkg_dir}/package.json")),
            content,
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from(format!("crates/{crate_dir}-node/package.json")),
            content: crate_pkg,
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from(format!("crates/{crate_dir}-node/index.js")),
            content: crate_index_js,
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from(format!("{pkg_dir}/src/index.d.ts")),
            content: dts_content.clone(),
            generated_header: false,
        },
        // Also emit at the package root so `package.json`'s `types: "index.d.ts"`
        // resolves for downstream consumers. The src/ copy is kept for in-tree
        // tsconfig "include": ["src"] tooling; the root copy is what npm exposes.
        GeneratedFile {
            path: PathBuf::from(format!("{pkg_dir}/index.d.ts")),
            content: dts_content,
            generated_header: false,
        },
        // Emit the runtime entry too — `package.json` declares `"main": "index.js"`
        // so consumers (vitest, node) need a CommonJS shim that re-exports the
        // napi-built bindings from the corresponding `crates/<name>-node/`.
        GeneratedFile {
            path: PathBuf::from(format!("{pkg_dir}/index.js")),
            content: js_content,
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from(format!("{pkg_dir}/tsconfig.json")),
            content: r#"{
  "compilerOptions": {
    "target": "ES2022",
    "lib": ["ES2022"],
    "module": "ESNext",
    "moduleResolution": "bundler",
    "strict": true,
    "noImplicitAny": true,
    "strictNullChecks": true,
    "strictFunctionTypes": true,
    "strictBindCallApply": true,
    "strictPropertyInitialization": true,
    "noImplicitThis": true,
    "useUnknownInCatchVariables": true,
    "alwaysStrict": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "noImplicitReturns": true,
    "noFallthroughCasesInSwitch": true,
    "noUncheckedIndexedAccess": true,
    "noImplicitOverride": true,
    "esModuleInterop": true,
    "allowSyntheticDefaultImports": true,
    "resolveJsonModule": true,
    "isolatedModules": true,
    "skipLibCheck": true,
    "forceConsistentCasingInFileNames": true,
    "noEmit": true
  },
  "include": ["src"]
}
"#
            .to_string(),
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from(format!("{pkg_dir}/.oxfmtrc.json")),
            content: r#"{
  "$schema": "./node_modules/oxfmt/configuration_schema.json",
  "printWidth": 120,
  "useTabs": true,
  "tabWidth": 4,
  "semi": true,
  "singleQuote": false,
  "trailingComma": "all",
  "arrowParens": "always",
  "endOfLine": "lf",
  "bracketSpacing": true,
  "sortImports": true,
  "sortPackageJson": true
}
"#
            .to_string(),
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from(format!("{pkg_dir}/.oxlintrc.json")),
            content: r#"{
  "$schema": "./node_modules/oxlint/configuration_schema.json",
  "categories": {
    "correctness": "error",
    "suspicious": "warn",
    "pedantic": "off",
    "perf": "warn",
    "style": "off",
    "restriction": "off"
  },
  "plugins": ["typescript", "import"],
  "env": {
    "es6": true,
    "node": true
  },
  "rules": {
    "no-console": "warn",
    "no-unused-vars": "warn",
    "typescript/no-explicit-any": "warn"
  },
  "overrides": [
    {
      "files": ["**/tests/**", "**/*.test.ts", "**/*.spec.ts"],
      "rules": {
        "no-console": "off",
        "no-unused-vars": "off",
        "typescript/no-explicit-any": "off"
      }
    }
  ]
}
"#
            .to_string(),
            generated_header: false,
        },
    ])
}
