use crate::{cargo_package_header, core_dep_features, detect_workspace_inheritance, render_extra_deps, scaffold_meta};
use alef_core::backend::GeneratedFile;
use alef_core::config::{AlefConfig, Language};
use alef_core::ir::ApiSurface;
use alef_core::template_versions as tv;
use std::path::PathBuf;

pub(crate) fn scaffold_node_cargo(api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let version = &api.version;
    let core_crate_dir = config.core_crate_dir();
    let ws = detect_workspace_inheritance(config.crate_config.workspace_root.as_deref());
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
    let extra_deps_section = if extra_deps.is_empty() {
        String::new()
    } else {
        format!("\n{extra_deps}")
    };
    let content = format!(
        r#"{pkg_header}

[lib]
crate-type = ["cdylib"]

[dependencies]
{crate_name} = {{ path = "../{core_crate_dir}"{features} }}
napi = {{ version = "{napi}", features = ["async"] }}
napi-derive = "{napi_derive}"
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"{extra_deps_section}

[build-dependencies]
napi-build = "{napi_build}"

[lints]
workspace = true
"#,
        pkg_header = pkg_header,
        crate_name = &config.crate_config.name,
        core_crate_dir = core_crate_dir,
        features = core_dep_features(config, Language::Node),
        napi = tv::cargo::NAPI,
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

pub(crate) fn scaffold_node(api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let package_name = config.node_package_name();
    let name = &config.crate_config.name;
    let version = &api.version;
    let pkg_dir = config.package_dir(Language::Node);

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
    let crate_dir = config.core_crate_dir();
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
