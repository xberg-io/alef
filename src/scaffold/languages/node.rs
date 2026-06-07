use crate::core::backend::GeneratedFile;
use crate::core::config::{AdapterPattern, Language, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, TypeRef};
use crate::core::template_versions as tv;
use crate::{
    scaffold::cargo_package_header, scaffold::core_dep_features, scaffold::detect_workspace_inheritance,
    scaffold::render_extra_deps, scaffold::scaffold_meta,
};
use std::path::PathBuf;

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
    config
        .node
        .as_ref()
        .map(|c| c.exclude_platforms.clone())
        .unwrap_or_default()
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

    // Build the cargo-machete ignored list. `serde_json` is always emitted
    // unconditionally above so we always ignore it. Conditional deps
    // (`async-trait` and `tokio-util` for trait bridges, `futures-util` for streaming) are
    // appended only when the scaffold actually adds them to `[dependencies]`,
    // so cargo-machete doesn't flap on umbrellas whose API surface doesn't
    // exercise the trait-bridge / streaming codepath.
    let mut machete_ignored: Vec<&str> = vec!["serde_json"];
    if has_trait_bridges {
        machete_ignored.push("async-trait");
        machete_ignored.push("tokio-util");
    }
    if has_streaming {
        machete_ignored.push("futures-util");
    }
    let machete_ignored_str = machete_ignored
        .iter()
        .map(|d| format!("\"{d}\""))
        .collect::<Vec<_>>()
        .join(", ");

    // Build [dependencies] block alphabetically sorted to match cargo-sort.
    // Order: async-trait?, futures-util?, <core-crate>, napi,
    // napi-derive, serde, serde_json, + any extra deps.
    let core_dep = crate::scaffold::render_core_dep(
        &config.name,
        &format!("../{core_crate_dir}"),
        &core_dep_features(config, Language::Node),
        version,
    );
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
    dep_entries.sort();
    let dep_block = dep_entries.join("\n");
    // Silence unused vars from the prior template wiring; the new dep block
    // collapses extra_deps_section into dep_entries directly.
    let _ = extra_deps_section;

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

[dependencies]
{dep_block}

[build-dependencies]
napi-build = "{napi_build}"

"#,
        pkg_header = pkg_header,
        dep_block = dep_block,
        machete_ignored_str = machete_ignored_str,
        napi_build = tv::cargo::NAPI_BUILD,
    );

    Ok(vec![GeneratedFile {
        path: PathBuf::from(format!("crates/{}-node/Cargo.toml", core_crate_dir)),
        content,
        generated_header: true,
    }])
}

fn generate_napi_platform_dispatch_index(binary_name: &str, package_name: &str, excluded: &[String]) -> String {
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

    format!(
        r#""use strict";

const {{ platform, arch }} = process;
const isWindows = platform === "win32";
const isMusl = () => {{
  // Prefer the report-header `glibcVersion` string when present — fastest and
  // unambiguous on Node builds that populate it. On Node 22+, certain CI
  // environments leave `glibcVersion` undefined even on glibc systems, so the
  // `=== undefined` branch from older napi-rs templates produces a false
  // "is musl" positive. Fall through to the filesystem heuristic instead: on
  // glibc systems `/lib64/ld-musl-x86_64.so.1` does not exist; on musl systems
  // it always does. statSync errors → not musl.
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
  throw new Error(
    `Failed to load native binding for ${{platform}}-${{arch}}. Errors: ${{loadErrors.join(", ")}}`
  );
}}

module.exports = nativeBinding;
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
    license: Option<&str>,
    repository_block: &str,
) -> String {
    let package_name = napi_platform_package_name(parent_package_name, platform);
    let (os, cpu, libc) = napi_platform_os_cpu_libc(platform);
    let libc_field = libc
        .map(|value| format!(",\n  \"libc\": [\"{value}\"]"))
        .unwrap_or_default();
    let binary_file = format!("{binary_name}.{platform}.node");
    let license_block = license
        .map(|value| format!(",\n  \"license\": \"{value}\""))
        .unwrap_or_default();

    format!(
        r#"{{
  "name": "{package_name}",
  "version": "{version}"{license_block}{repository_block},
  "main": "{binary_file}",
  "files": ["{binary_file}"],
  "os": ["{os}"],
  "cpu": ["{cpu}"]{libc_field},
  "engines": {{ "node": ">= 18" }},
  "publishConfig": {{ "access": "public" }}
}}
"#,
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

    // Crate-level package.json required by `napi build`
    let crate_pkg = format!(
        r#"{{
  "name": "{package_name}",
  "version": "{version}",
  "description": "{description}"{license_block}{repository_block},
  "main": "index.js",
  "types": "index.d.ts",
  "exports": {{
    ".": {{
      "types": "./index.d.ts",
      "require": "./index.js",
      "default": "./index.js"
    }}
  }},
  "files": ["index.js", "index.d.ts", "*.node"],
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
    "build": "npx --yes @napi-rs/cli@{napi_rs_cli_crate} build --platform --release",
    "artifacts": "npx --yes @napi-rs/cli@{napi_rs_cli_crate} artifacts",
    "prepublishOnly": "npx --yes @napi-rs/cli@{napi_rs_cli_crate} prepublish -t npm --skip-optional-publish"
  }},
  "engines": {{ "node": ">= 18" }},
  "publishConfig": {{ "access": "public" }},
  "devDependencies": {{ "@napi-rs/cli": "{napi_rs_cli_crate}" }}
}}
"#,
        package_name = package_name,
        version = version,
        description = meta.description,
        license_block = license_block,
        repository_block = repository_block,
        crate_dir = crate_dir,
        optional_dependencies = optional_dependencies,
        targets = targets,
        napi_rs_cli_crate = tv::npm::NAPI_RS_CLI_CRATE,
    );

    let crate_index_js =
        generate_napi_platform_dispatch_index(&format!("{}-node", crate_dir), &package_name, &excluded);
    let binary_name = format!("{crate_dir}-node");

    // The npm publish target lives at `crates/{crate_dir}-node/` and is built by
    // NAPI-RS. We only emit the crate-level `package.json` + platform-dispatch
    // `index.js` here — the historical `packages/node/` scaffold was dead weight
    // that defined a parallel unscoped npm package that was never published.
    let mut files = vec![
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
    ];
    files.extend(active_platforms.iter().map(|(platform, _)| GeneratedFile {
        path: PathBuf::from(format!("crates/{crate_dir}-node/npm/{platform}/package.json")),
        content: generate_napi_platform_package_json(
            &package_name,
            &binary_name,
            platform,
            version,
            meta.license.as_deref(),
            &repository_block,
        ),
        generated_header: false,
    }));
    Ok(files)
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
