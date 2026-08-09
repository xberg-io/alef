use crate::codegen::cfg as shared_cfg;
use crate::core::backend::GeneratedFile;
use crate::core::config::{AdapterPattern, Language, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, TypeRef};
use crate::core::template_versions as tv;
use crate::{
    scaffold::ScaffoldMeta, scaffold::cargo_package_header, scaffold::detect_workspace_inheritance_for_crate,
    scaffold::render_extra_deps, scaffold::scaffold_meta,
};
use anyhow::Context as _;
use std::path::{Path, PathBuf};

const NAPI_TARGETS: &[&str] = &[
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-unknown-linux-musl",
    "aarch64-unknown-linux-musl",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
    "x86_64-pc-windows-msvc",
    "aarch64-pc-windows-msvc",
];

const NAPI_PLATFORMS: &[&str] = &[
    "linux-x64-gnu",
    "linux-arm64-gnu",
    "linux-x64-musl",
    "linux-arm64-musl",
    "darwin-x64",
    "darwin-arm64",
    "win32-x64-msvc",
    "win32-arm64-msvc",
];

/// Runtime dispatch metadata for the platform loader (`index.js`).
/// One row per [`NAPI_PLATFORMS`] entry, in lockstep order with
/// [`NAPI_TARGETS`]. Tuple is `(platform_key, process.platform, process.arch,
/// libc_abi)`. `libc_abi` is `None` for darwin/windows where libc detection
/// is unused.
const NAPI_DISPATCH_ROWS: &[(&str, &str, &str, Option<&str>)] = &[
    ("linux-x64-gnu", "linux", "x64", Some("gnu")),
    ("linux-arm64-gnu", "linux", "arm64", Some("gnu")),
    ("linux-x64-musl", "linux", "x64", Some("musl")),
    ("linux-arm64-musl", "linux", "arm64", Some("musl")),
    ("darwin-x64", "darwin", "x64", None),
    ("darwin-arm64", "darwin", "arm64", None),
    ("win32-x64-msvc", "win32", "x64", None),
    ("win32-arm64-msvc", "win32", "arm64", None),
];

fn excluded_node_platforms(config: &ResolvedCrateConfig) -> Vec<String> {
    let mut excluded = config
        .node
        .as_ref()
        .map(|c| c.exclude_platforms.clone())
        .unwrap_or_default();
    // Also drop any napi platform whose target triple is disabled via the ~keep
    // workspace `[targets]` opt-out table, so a single toggle stays consistent ~keep
    // with the language-specific `exclude_platforms` list. ~keep
    for (platform, triple) in NAPI_PLATFORMS.iter().zip(NAPI_TARGETS.iter()) {
        if !config.target_enabled(triple) && !excluded.iter().any(|e| e == platform) {
            excluded.push((*platform).to_string());
        }
    }
    excluded
}

fn napi_platforms_filtered(excluded: &[String]) -> Vec<(&'static str, &'static str)> {
    NAPI_PLATFORMS
        .iter()
        .zip(NAPI_TARGETS.iter())
        .filter(|(p, _)| !excluded.iter().any(|e| e == *p))
        .map(|(p, t)| (*p, *t))
        .collect()
}

fn napi_dispatch_rows_filtered(
    excluded: &[String],
) -> Vec<(&'static str, &'static str, &'static str, Option<&'static str>)> {
    NAPI_DISPATCH_ROWS
        .iter()
        .filter(|(p, ..)| !excluded.iter().any(|e| e == *p))
        .copied()
        .collect()
}

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
    for type_def in &api.types {
        for field in &type_def.fields {
            if type_ref_contains_json(&field.ty) {
                return true;
            }
        }
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
    let crate_dir = format!("crates/{core_crate_dir}-node");
    let ws = detect_workspace_inheritance_for_crate(config.workspace_root.as_deref(), &crate_dir);
    let pkg_header = cargo_package_header(&format!("{core_crate_dir}-node"), version, "2024", &meta, &ws);

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
    if has_trait_bridges && !all_deps.contains("tokio-util") {
        if !all_deps.is_empty() {
            all_deps.push('\n');
        }
        let tokio_util_feats = config
            .node
            .as_ref()
            .and_then(|n| n.tokio_util_features.as_ref())
            .cloned()
            .unwrap_or_else(|| vec!["rt".to_string()]);
        let feats_list = tokio_util_feats
            .iter()
            .map(|f| format!("\"{f}\""))
            .collect::<Vec<_>>()
            .join(", ");
        all_deps.push_str(&format!(
            "tokio-util = {{ version = \"0.7\", features = [{feats_list}] }}"
        ));
    }
    if has_streaming && !all_deps.contains("futures-util = ") && !all_deps.contains("futures-util =\"") {
        if !all_deps.is_empty() {
            all_deps.push('\n');
        }
        all_deps.push_str("futures-util = \"0.3\"");
    }
    if has_trait_bridges && !all_deps.contains("tracing") {
        if !all_deps.is_empty() {
            all_deps.push('\n');
        }
        all_deps.push_str(&format!("tracing = \"{}\"", tv::cargo::TRACING));
    }
    if !config.components.is_empty() {
        let alef_version = env!("CARGO_PKG_VERSION");
        for dependency in [
            ("alef-component-abi", format!("alef-component-abi = \"{alef_version}\"")),
            (
                "alef-component-runtime",
                format!("alef-component-runtime = \"{alef_version}\""),
            ),
            ("directories", "directories = \"6\"".to_string()),
        ] {
            if crate::scaffold::cargo_dependency_declared(all_deps.lines(), dependency.0) {
                continue;
            }
            if !all_deps.is_empty() {
                all_deps.push('\n');
            }
            all_deps.push_str(&dependency.1);
        }
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

    let mut machete_ignored: Vec<&str> = vec!["serde_json"];
    if has_trait_bridges {
        machete_ignored.push("async-trait");
        machete_ignored.push("tokio-util");
        machete_ignored.push("tracing");
    }
    if has_streaming {
        machete_ignored.push("futures-util");
    }
    let machete_ignored_str = machete_ignored
        .iter()
        .map(|d| format!("\"{d}\""))
        .collect::<Vec<_>>()
        .join(", ");

    let core_overrides = config
        .node
        .as_ref()
        .map(|c| c.target_dep_overrides.as_slice())
        .unwrap_or(&[]);
    let excluded_default_features: std::collections::HashSet<&str> = config
        .node
        .as_ref()
        .map(|c| c.excluded_default_features.iter().map(String::as_str).collect())
        .unwrap_or_default();
    let core_dep_path = config.core_crate_dep_path(std::path::Path::new(&crate_dir));
    let (core_dep, core_target_blocks) = crate::scaffold::render_core_dep_with_overrides(
        &config.name,
        &core_dep_path,
        &crate::scaffold::core_dep_features_excluding(config, Language::Node, &excluded_default_features),
        version,
        core_overrides,
    );
    let core_target_blocks_section = if core_target_blocks.is_empty() {
        String::new()
    } else {
        format!("{core_target_blocks}\n")
    };
    let mut dep_entries: Vec<String> = vec![
        format!(
            "napi = {{ version = \"{napi}\", features = [{napi_features_str}] }}",
            napi = tv::cargo::NAPI
        ),
        format!("napi-derive = \"{}\"", tv::cargo::NAPI_DERIVE),
        "serde = { version = \"1\", features = [\"derive\"] }".to_string(),
        "serde_json = \"1\"".to_string(),
    ];
    if !core_dep.is_empty() {
        dep_entries.push(core_dep.clone());
    }
    if !all_deps.is_empty() {
        for line in all_deps.lines() {
            if !line.is_empty() {
                dep_entries.push(line.to_string());
            }
        }
    }
    crate::scaffold::sort_dependency_lines(&mut dep_entries);
    let dep_block = dep_entries.join("\n");
    let _ = extra_deps_section;

    // `#[cfg(feature = "X")]` arms emitted by the codegen produce
    let mut cfg_features = shared_cfg::collect_cfg_features(api);
    // A config-only `excluded_default_features` name (gates no `#[cfg(feature = ...)]`) must
    // still get a forwarding entry below -- alef-task #374, regression in
    // `cargo_excluded_features_tests`. ~keep
    cfg_features.extend(excluded_default_features.iter().map(|name| (*name).to_string()));
    // A name in `excluded_default_features` is still declared below (so `cargo build --features
    // <name>` keeps working) but dropped from `default`, matching
    // `RubyConfig::excluded_default_features`. ~keep
    let features_table = if cfg_features.is_empty() {
        String::new()
    } else {
        let lines =
            shared_cfg::cfg_default_and_forwarding_lines(&cfg_features, &config.name, &excluded_default_features);
        format!("[features]\n{}\n\n", lines.join("\n"))
    };

    let lints_section = crate::scaffold::cargo_lints_section(config);
    let content = format!(
        r#"{pkg_header}

# `serde_json` is emitted unconditionally above so the manifest is stable
# across regens, but for umbrella crates with no JSON-marshalled return types
# it is genuinely unused. The conditional `async-trait` / `futures-util` deps
# are similarly flagged when the umbrella has trait-bridge / streaming
# adapters configured but no actual async-trait callsite in this binding.
[package.metadata.cargo-machete]
ignored = [{machete_ignored_str}]

[lib]
crate-type = ["cdylib"]

{features_table}[dependencies]
{dep_block}

{core_target_blocks_section}[build-dependencies]
napi-build = "{napi_build}"
{lints_section}"#,
        pkg_header = pkg_header,
        lints_section = lints_section,
        dep_block = dep_block,
        core_target_blocks_section = core_target_blocks_section,
        features_table = features_table,
        machete_ignored_str = machete_ignored_str,
        napi_build = tv::cargo::NAPI_BUILD,
    );

    Ok(vec![GeneratedFile {
        path: PathBuf::from(format!("crates/{}-node/Cargo.toml", core_crate_dir)),
        content,
        generated_header: true,
    }])
}

fn generate_napi_platform_dispatch_index(
    binary_name: &str,
    package_name: &str,
    excluded: &[String],
    has_service_api: bool,
) -> String {
    let rows = napi_dispatch_rows_filtered(excluded);
    let targets_lines = rows
        .iter()
        .map(|(platform_key, plat, arch, abi)| {
            let abi_token = match abi {
                Some(a) => format!("\"{a}\""),
                None => "null".to_string(),
            };
            format!(
                "    [\"{plat}\", \"{arch}\", {abi_token}, \"./{binary_name}.{platform_key}.node\", \"{package_name}-{platform_key}\"],"
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let export_statement = if has_service_api {
        r#"const _service = require("./service.cjs");
module.exports = { ...nativeBinding, ..._service };"#
    } else {
        "module.exports = nativeBinding;"
    };

    format!(
        r#""use strict";

const {{ platform, arch }} = process;
const isMusl = () => {{
  // Prefer the report-header `glibcVersion` string when present — fastest and
  // unambiguous on Node builds that populate it. On Node 22+, certain CI
  // environments leave `glibcVersion` undefined even on glibc systems, so the
  // `=== undefined` branch from older napi-rs templates produces a false
  // "is musl" positive. Fall through to the filesystem heuristic instead: on
  // glibc systems `/lib64/ld-musl-x86_64.so.1` does not exist; on musl systems
  // it always does. statSync errors → not musl.
  if (typeof process.report === "object" && typeof process.report.getReport === "function") {{
    const report = process.report.getReport();
    if (report && report.header && typeof report.header.glibcVersion === "string") {{
      return false;
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
  // Local `.node` files are named after `napi.binaryName` (binary file name on disk).
  // Optional-dep packages are named after `napi.packageName` (npm subpackage names),
  // which inherits any scope prefix from the parent package.
  const targets = [
{targets_lines}
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
  throw new Error(`Failed to load native binding for ${{platform}}-${{arch}}. Errors: ${{loadErrors.join(", ")}}`);
}}

{export_statement}
"#,
    )
}

fn napi_platform_package_name(parent_package_name: &str, platform: &str) -> String {
    format!("{parent_package_name}-{platform}")
}

fn napi_platform_os_cpu_libc(platform: &str) -> (&'static str, &'static str, Option<&'static str>) {
    match platform {
        "linux-x64-gnu" => ("linux", "x64", Some("glibc")),
        "linux-arm64-gnu" => ("linux", "arm64", Some("glibc")),
        "linux-x64-musl" => ("linux", "x64", Some("musl")),
        "linux-arm64-musl" => ("linux", "arm64", Some("musl")),
        "darwin-x64" => ("darwin", "x64", None),
        "darwin-arm64" => ("darwin", "arm64", None),
        "win32-x64-msvc" => ("win32", "x64", None),
        "win32-arm64-msvc" => ("win32", "arm64", None),
        _ => ("linux", "x64", None),
    }
}

fn generate_napi_platform_package_json(
    parent_package_name: &str,
    binary_name: &str,
    platform: &str,
    version: &str,
    meta: &ScaffoldMeta,
    repository_block: &str,
) -> String {
    let package_name = napi_platform_package_name(parent_package_name, platform);
    let (os, cpu, libc) = napi_platform_os_cpu_libc(platform);
    let libc_field = libc
        .map(|value| format!(",\n  \"libc\": [\"{value}\"]"))
        .unwrap_or_default();
    let binary_file = format!("{binary_name}.{platform}.node");
    let license_block = meta
        .license
        .as_deref()
        .map(|value| format!(",\n  \"license\": \"{value}\""))
        .unwrap_or_default();
    let (homepage_block, bugs_block, author_block, keywords_block) = npm_publish_metadata_blocks(meta);

    format!(
        r#"{{
  "name": "{package_name}",
  "version": "{version}"{license_block}{repository_block}{homepage_block}{bugs_block}{author_block}{keywords_block},
  "main": "{binary_file}",
  "files": ["{binary_file}"],
  "os": ["{os}"],
  "cpu": ["{cpu}"]{libc_field},
  "engines": {{ "node": "{node_engine}" }},
  "publishConfig": {{ "access": "public" }}
}}
"#,
        node_engine = tv::npm::NODE_ENGINE,
    )
}

pub(crate) fn scaffold_node(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let package_name = config.node_package_name();
    let version = &api.version;
    let crate_dir = config.core_crate_dir();

    let repository_block = meta
        .configured_repository
        .as_deref()
        .map(npm_repository_block)
        .unwrap_or_default();
    let license_block = meta
        .license
        .as_deref()
        .map(|license| format!(",\n  \"license\": \"{license}\""))
        .unwrap_or_default();
    let (homepage_block, bugs_block, author_block, keywords_block) = npm_publish_metadata_blocks(&meta);
    let excluded = excluded_node_platforms(config);
    let active_platforms = napi_platforms_filtered(&excluded);
    let optional_dependencies = active_platforms
        .iter()
        .map(|(platform, _)| {
            format!(
                "    \"{}\": \"{}\"",
                napi_platform_package_name(&package_name, platform),
                version
            )
        })
        .collect::<Vec<_>>()
        .join(",\n");
    let targets = active_platforms
        .iter()
        .map(|(_, target)| format!("      \"{target}\""))
        .collect::<Vec<_>>()
        .join(",\n");

    let has_service_api = !api.services.is_empty();
    let entrypoint = if has_service_api {
        "index-wrapper.cjs"
    } else {
        "index.js"
    };
    let files_list = if has_service_api {
        "[\"index.js\", \"index-wrapper.cjs\", \"index.d.ts\", \"service.cjs\", \"*.node\"]".to_string()
    } else {
        "[\"index.js\", \"index.d.ts\", \"*.node\"]".to_string()
    };
    // When a service API is present, `service.cjs` is a real module (the low-level ~keep
    // service `App`); expose it as a `./service` subpath so consumers can ~keep
    // `import ... from '<pkg>/service'`. Without this the subpath is unresolvable ~keep
    // under the `exports` map even though the file ships in `files`. ~keep
    let exports_map = if has_service_api {
        format!(
            r#"{{
    ".": {{
      "types": "./index.d.ts",
      "require": "./{entrypoint}",
      "default": "./{entrypoint}"
    }},
    "./service": {{
      "types": "./index.d.ts",
      "require": "./service.cjs",
      "default": "./service.cjs"
    }}
  }}"#
        )
    } else {
        format!(
            r#"{{
    ".": {{
      "types": "./index.d.ts",
      "require": "./{entrypoint}",
      "default": "./{entrypoint}"
    }}
  }}"#
        )
    };

    let crate_pkg = format!(
        r#"{{
  "name": "{package_name}",
  "version": "{version}",
  "description": "{description}"{license_block}{repository_block}{homepage_block}{bugs_block}{author_block}{keywords_block},
  "main": "{entrypoint}",
  "types": "index.d.ts",
  "exports": {exports_map},
  "files": {files_list},
  "optionalDependencies": {{
{optional_dependencies}
  }},
  "napi": {{
    "packageName": "{package_name}",
    "binaryName": "{crate_dir}-node",
    "targets": [
{targets}
    ]
  }},
  "scripts": {{
    "build": "npx --yes -p @napi-rs/cli@{napi_rs_cli_crate} napi build --platform --no-js --dts {napi_dts} --release",
    "artifacts": "npx --yes -p @napi-rs/cli@{napi_rs_cli_crate} napi artifacts",
    "prepublishOnly": "npx --yes -p @napi-rs/cli@{napi_rs_cli_crate} napi prepublish -t npm --skip-optional-publish"
  }},
  "engines": {{ "node": "{node_engine}" }},
  "publishConfig": {{ "access": "public" }},
  "devDependencies": {{ "@napi-rs/cli": "{napi_rs_cli_crate}" }}
}}
"#,
        package_name = package_name,
        version = version,
        description = meta.description,
        license_block = license_block,
        repository_block = repository_block,
        homepage_block = homepage_block,
        bugs_block = bugs_block,
        author_block = author_block,
        keywords_block = keywords_block,
        crate_dir = crate_dir,
        exports_map = exports_map,
        files_list = files_list,
        optional_dependencies = optional_dependencies,
        targets = targets,
        napi_rs_cli_crate = tv::npm::NAPI_RS_CLI_CRATE,
        napi_dts = tv::npm::NAPI_AUTO_DTS_FILENAME,
        node_engine = tv::npm::NODE_ENGINE,
        entrypoint = entrypoint,
    );

    let crate_index_js = generate_napi_platform_dispatch_index(
        &format!("{}-node", crate_dir),
        &package_name,
        &excluded,
        has_service_api,
    );
    let binary_name = format!("{crate_dir}-node");

    let mut files = vec![
        GeneratedFile {
            path: PathBuf::from(format!("crates/{crate_dir}-node/package.json")),
            content: crate_pkg,
            generated_header: false,
        },
        // `generated_header: false` -- user-owned after this scaffold. `napi build`'s own
        // `--js` output filename defaults to `index.js` too, and (unlike `--dts`) napi-rs
        // supports disabling that output entirely with `--no-js` (see the `"build"` script
        // above and `build_command_for`'s `"napi"` arm), rather than needing a redirect: this
        // hand-derived platform-dispatch file already does everything napi-rs's own auto
        // dispatch file would, so nothing consumes the napi-rs version. Without `--no-js`,
        // every `napi build --platform` invocation -- the default node build step, `alef
        // publish`, and the scaffolded `npm run build` -- would silently overwrite this file. ~keep
        GeneratedFile {
            path: PathBuf::from(format!("crates/{crate_dir}-node/index.js")),
            content: crate_index_js,
            generated_header: false,
        },
    ];
    files.push(GeneratedFile {
        // `--dts index.native.d.ts` points napi-rs's auto-derived declarations away from
        // `index.d.ts`, which alef owns. The redirect target is a pure build artifact: it is
        // absent from `files` so npm never receives it, no `types`/`exports` entry names it,
        // and nothing imports it. Without this, every `npm run build` drops an untracked file
        // into the consumer's tree. ~keep
        path: PathBuf::from(format!("crates/{crate_dir}-node/.gitignore")),
        content: format!("{}\n", tv::npm::NAPI_AUTO_DTS_FILENAME),
        generated_header: false,
    });
    if has_service_api {
        files.push(GeneratedFile {
            path: PathBuf::from(format!("crates/{crate_dir}-node/index-wrapper.cjs")),
            content: "// DO NOT EDIT - auto-generated by alef\n\"use strict\";\nconst _native = require(\"./index.js\");\nconst _service = require(\"./service.cjs\");\nmodule.exports = { ..._native, ..._service };\n".to_string(),
            generated_header: false,
        });
    }
    files.extend(active_platforms.iter().map(|(platform, _)| GeneratedFile {
        path: PathBuf::from(format!("crates/{crate_dir}-node/npm/{platform}/package.json")),
        content: generate_napi_platform_package_json(
            &package_name,
            &binary_name,
            platform,
            version,
            &meta,
            &repository_block,
        ),
        generated_header: false,
    }));
    Ok(files)
}

/// The exact `exports` map [`scaffold_node`] emitted for a service-API crate before the fix
/// that exposed a `./service` subpath. `entrypoint` is a fixed literal (`index-wrapper.cjs`)
/// whenever `has_service_api` is true, so this whole block carries no per-project variables at
/// all — every service-API crate scaffolded before the fix got this identical text.
const STALE_SERVICE_EXPORTS_MAP: &str = "\"exports\": {\n    \".\": {\n      \"types\": \"./index.d.ts\",\n      \"require\": \"./index-wrapper.cjs\",\n      \"default\": \"./index-wrapper.cjs\"\n    }\n  },";

/// The replacement `exports` map for a service-API crate, adding the `./service` subpath —
/// the exact shape [`scaffold_node`]'s `has_service_api` branch emits today.
const FIXED_SERVICE_EXPORTS_MAP: &str = "\"exports\": {\n    \".\": {\n      \"types\": \"./index.d.ts\",\n      \"require\": \"./index-wrapper.cjs\",\n      \"default\": \"./index-wrapper.cjs\"\n    },\n    \"./service\": {\n      \"types\": \"./index.d.ts\",\n      \"require\": \"./service.cjs\",\n      \"default\": \"./service.cjs\"\n    }\n  },";

/// Repair a pre-existing `crates/<crate>-node/package.json` whose `exports` map still lacks the
/// `./service` subpath for a service-API crate — the exact defect fixed in [`scaffold_node`]'s
/// `has_service_api` branch (a `./service` subpath export was added so consumers can
/// `import ... from '<pkg>/service'`).
///
/// `crates/*-node/package.json` is `generated_header: false` (create-only), so a service-API
/// crate scaffolded before that fix keeps shipping `service.cjs` in `files` (the module ships)
/// while `exports` never resolves the `./service` subpath at all — the file is present but
/// entirely unreachable via `require`/`import`. Detection requires *both* the exact stale
/// `exports` block text (see [`STALE_SERVICE_EXPORTS_MAP`], which — unlike `main`/`module`/
/// `types` in the wasm package.json case — carries no per-project variables, so an exact
/// substring match is sufficient) *and* `"service.cjs"` present in the file's `files` array, as
/// independent confirmation this really is a service-API crate's package.json and not some
/// coincidental byte match. Only a single occurrence of the stale block is ever replaced — a
/// package.json with the block duplicated (which `scaffold_node` never produces) is left
/// untouched rather than guessed at. Everything else in the file — a consumer's added
/// `devDependencies`, extra `scripts`, reordered fields — survives byte-for-byte, since this
/// only ever replaces the one matched substring. ~keep
pub(crate) fn migrate_node_package_json_service_export(base_dir: &Path, relative_path: &Path) -> anyhow::Result<bool> {
    let path = crate::cli::pipeline::generate::write::contained_output_path(base_dir, relative_path)?;
    let Ok(existing) = std::fs::read_to_string(&path) else {
        return Ok(false);
    };
    if !existing.contains("\"service.cjs\"") {
        return Ok(false);
    }
    if existing.matches(STALE_SERVICE_EXPORTS_MAP).count() != 1 {
        return Ok(false);
    }
    let migrated = existing.replacen(STALE_SERVICE_EXPORTS_MAP, FIXED_SERVICE_EXPORTS_MAP, 1);

    let parent = path
        .parent()
        .context("node package.json path has no parent directory")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("failed to create temporary file in {}", parent.display()))?;
    std::io::Write::write_all(&mut temporary, migrated.as_bytes())
        .with_context(|| format!("failed to write temporary file for {}", path.display()))?;
    temporary
        .persist(&path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to replace {}", path.display()))?;
    // Fires only after the replace above already succeeded: a completed self-heal, not an
    // outstanding problem. ~keep
    tracing::info!(
        path = %path.display(),
        "repaired pre-existing crates/*-node/package.json: exposed the ./service subpath export"
    );
    Ok(true)
}

fn npm_repository_block(repository_url: &str) -> String {
    let repository_git_url = if repository_url.starts_with("git+") {
        repository_url.to_string()
    } else {
        format!(
            "git+{}.git",
            repository_url.trim_end_matches('/').trim_end_matches(".git")
        )
    };
    format!(
        r#",
  "repository": {{
    "type": "git",
    "url": "{repository_git_url}"
  }}"#
    )
}

/// Build the `homepage` / `bugs` / `author` / `contributors` / `keywords` blocks
/// shared by the crate package.json and each per-platform manifest, each already
/// prefixed with a leading `,\n  "key": ...` for direct splicing after the
/// existing `license_block`/`repository_block` fields. Empty when the
/// corresponding [`ScaffoldMeta`] field is unset, matching the license/repository
/// block convention already in place.
fn npm_publish_metadata_blocks(meta: &ScaffoldMeta) -> (String, String, String, String) {
    let homepage_block = if meta.homepage.is_empty() {
        String::new()
    } else {
        format!(",\n  \"homepage\": \"{}\"", meta.homepage)
    };
    let bugs_block = if meta.issues.is_empty() {
        String::new()
    } else {
        format!(",\n  \"bugs\": {{\n    \"url\": \"{}\"\n  }}", meta.issues)
    };
    // npm's package.json schema treats `author` as a single Person and
    // `contributors` as the array for everyone else. ~keep
    let author_block = match meta.authors.split_first() {
        None => String::new(),
        Some((primary, rest)) => {
            let primary_block = format!(",\n  \"author\": \"{primary}\"");
            if rest.is_empty() {
                primary_block
            } else {
                let entries: Vec<String> = rest.iter().map(|a| format!("\"{a}\"")).collect();
                format!("{primary_block},\n  \"contributors\": [{}]", entries.join(", "))
            }
        }
    };
    let keywords_block = if meta.keywords.is_empty() {
        String::new()
    } else {
        let entries: Vec<String> = meta.keywords.iter().map(|k| format!("\"{k}\"")).collect();
        format!(",\n  \"keywords\": [{}]", entries.join(", "))
    };
    (homepage_block, bugs_block, author_block, keywords_block)
}

#[cfg(test)]
mod migrate_tests {
    use super::*;

    fn pre_fix_service_package_json() -> String {
        format!(
            "{{\n  \"name\": \"@scope/example\",\n  \"version\": \"1.0.0\",\n  \"main\": \"index-wrapper.cjs\",\n  \"types\": \"index.d.ts\",\n  {STALE_SERVICE_EXPORTS_MAP}\n  \"files\": [\"index.js\", \"index-wrapper.cjs\", \"index.d.ts\", \"service.cjs\", \"*.node\"],\n  \"scripts\": {{\n    \"build\": \"napi build\"\n  }}\n}}\n"
        )
    }

    #[test]
    fn should_add_service_subpath_export_when_missing_from_service_api_crate() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pkg_dir = dir.path().join("crates/example-node");
        std::fs::create_dir_all(&pkg_dir).expect("create crates/example-node");
        std::fs::write(pkg_dir.join("package.json"), pre_fix_service_package_json())
            .expect("write pre-fix package.json");

        let relative_path = Path::new("crates/example-node/package.json");
        let changed =
            migrate_node_package_json_service_export(dir.path(), relative_path).expect("migration must not error");
        assert!(
            changed,
            "a service-API package.json missing ./service export must be reported as changed"
        );

        let on_disk = std::fs::read_to_string(pkg_dir.join("package.json")).expect("read migrated file");
        let parsed: serde_json::Value = serde_json::from_str(&on_disk).expect("migrated file must be valid JSON");
        assert_eq!(parsed["exports"]["./service"]["require"], "./service.cjs");
        assert_eq!(
            parsed["exports"]["."]["require"], "./index-wrapper.cjs",
            "the root export must be preserved"
        );
        assert_eq!(
            parsed["scripts"]["build"], "napi build",
            "fields outside exports must survive untouched"
        );

        let changed_again =
            migrate_node_package_json_service_export(dir.path(), relative_path).expect("second pass must not error");
        assert!(
            !changed_again,
            "second pass over an already-migrated file must be a no-op"
        );
    }

    #[test]
    fn should_not_touch_a_non_service_package_json() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pkg_dir = dir.path().join("crates/example-node");
        std::fs::create_dir_all(&pkg_dir).expect("create crates/example-node");
        let non_service = "{\n  \"name\": \"@scope/example\",\n  \"main\": \"index.js\",\n  \"exports\": {\n    \".\": {\n      \"types\": \"./index.d.ts\",\n      \"require\": \"./index.js\",\n      \"default\": \"./index.js\"\n    }\n  },\n  \"files\": [\"index.js\", \"index.d.ts\", \"*.node\"]\n}\n";
        std::fs::write(pkg_dir.join("package.json"), non_service).expect("write non-service package.json");

        let relative_path = Path::new("crates/example-node/package.json");
        let changed =
            migrate_node_package_json_service_export(dir.path(), relative_path).expect("migration must not error");
        assert!(
            !changed,
            "a package.json with no service.cjs in files must never be touched"
        );

        let on_disk = std::fs::read_to_string(pkg_dir.join("package.json")).expect("read file");
        assert_eq!(
            on_disk, non_service,
            "a non-service package.json must survive byte-for-byte"
        );
    }

    #[test]
    fn should_not_touch_a_package_json_that_already_has_the_service_export() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pkg_dir = dir.path().join("crates/example-node");
        std::fs::create_dir_all(&pkg_dir).expect("create crates/example-node");
        let already_fixed = format!(
            "{{\n  \"name\": \"@scope/example\",\n  {FIXED_SERVICE_EXPORTS_MAP}\n  \"files\": [\"index.js\", \"index-wrapper.cjs\", \"index.d.ts\", \"service.cjs\", \"*.node\"]\n}}\n"
        );
        std::fs::write(pkg_dir.join("package.json"), &already_fixed).expect("write already-fixed package.json");

        let relative_path = Path::new("crates/example-node/package.json");
        let changed =
            migrate_node_package_json_service_export(dir.path(), relative_path).expect("migration must not error");
        assert!(
            !changed,
            "a package.json that already has the ./service export must never be touched"
        );

        let on_disk = std::fs::read_to_string(pkg_dir.join("package.json")).expect("read file");
        assert_eq!(
            on_disk, already_fixed,
            "an already-fixed package.json must survive byte-for-byte"
        );
    }

    #[test]
    fn migrate_node_service_export_is_a_no_op_when_file_does_not_exist() {
        let dir = tempfile::tempdir().expect("tempdir");
        let relative_path = Path::new("crates/example-node/package.json");
        let changed = migrate_node_package_json_service_export(dir.path(), relative_path).expect("must not error");
        assert!(!changed);
        assert!(!dir.path().join(relative_path).exists());
    }
}

/// Regression for alef-task #374: an `excluded_default_features` name that gates no item in the
/// extracted API surface (e.g. a Cargo-only feature that only affects a dependency's `build.rs`
/// linking, such as `libheif-sys` via `heic`) is never discovered by
/// `shared_cfg::collect_cfg_features`, which walks `#[cfg(feature = "X")]` attributes on IR
/// nodes. The `[features]` table was built exclusively from that discovery set, so a
/// config-only name never got its promised opt-in forwarding entry at all -- breaking
/// `cargo build -p <crate>-node --features <name>` on desktop, exactly the escape hatch
/// `excluded_default_features` documents as always available. A test using a name that IS
/// cfg-discoverable would only exercise the already-working half.
#[cfg(test)]
mod cargo_excluded_features_tests {
    use super::*;
    use crate::core::config::NewAlefConfig;

    #[test]
    fn scaffold_node_cargo_forwards_excluded_feature_not_referenced_by_any_cfg_attribute() {
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["node"]
[[crates]]
name = "sample-lib"
sources = []
[crates.node]
excluded_default_features = ["heic"]
"#,
        )
        .expect("valid config");
        let config = cfg.resolve().expect("resolve").remove(0);
        let api = ApiSurface {
            crate_name: "sample-lib".to_string(),
            version: "1.0.0".to_string(),
            ..Default::default()
        };

        let files = scaffold_node_cargo(&api, &config).expect("scaffold_node_cargo ok");
        let cargo_toml = &files
            .iter()
            .find(|f| f.path.to_string_lossy().ends_with("Cargo.toml"))
            .expect("Cargo.toml emitted")
            .content;

        assert!(
            cargo_toml.contains("[features]"),
            "a config-only excluded_default_features name must still produce a [features] table:\n{cargo_toml}"
        );
        assert!(
            cargo_toml.contains(r#"heic = ["sample-lib/heic"]"#),
            "a config-only excluded_default_features name (not referenced by any \
             #[cfg(feature = ...)] in the API surface) must still get a forwarding entry so \
             `cargo build --features heic` keeps working:\n{cargo_toml}"
        );
        let default_line = cargo_toml
            .lines()
            .find(|line| line.starts_with("default = ["))
            .expect("default array present");
        assert!(
            !default_line.contains("heic"),
            "default = [...] must NOT contain excluded `heic`; got: {default_line}"
        );
    }
}
