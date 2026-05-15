use alef_core::backend::GeneratedFile;
use alef_core::config::ResolvedCrateConfig;
use alef_core::ir::{ApiSurface, TypeRef};
use std::path::PathBuf;

fn type_has_json(t: &TypeRef) -> bool {
    match t {
        TypeRef::Json => true,
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => type_has_json(inner),
        TypeRef::Map(k, v) => type_has_json(k) || type_has_json(v),
        _ => false,
    }
}

/// Returns true when the IR surface contains a TypeRef::Json field OR when any
/// Named field resolves to an enum type. The dart bridge codegen emits
/// `serde_json::to_string(&enum_value)` for enum-typed fields (they are not
/// FRB-primitive but need serialisation for the JSON helper functions), so
/// `serde_json` must appear in the bridge Cargo.toml whenever either condition holds.
fn api_has_json_or_enum_field(api: &ApiSurface) -> bool {
    if api
        .types
        .iter()
        .flat_map(|t| t.fields.iter())
        .any(|f| type_has_json(&f.ty))
        || api
            .functions
            .iter()
            .any(|f| f.params.iter().any(|p| type_has_json(&p.ty)) || type_has_json(&f.return_type))
    {
        return true;
    }

    // D6: also return true when any non-opaque, non-trait struct has a Named field
    // whose resolved type is an enum. The bridge emits `serde_json::to_string` for
    // those fields in the `From<CoreT>` impls and the `create_*_from_json` helpers.
    let enum_names: std::collections::HashSet<&str> = api.enums.iter().map(|e| e.name.as_str()).collect();

    fn type_ref_contains_enum(t: &TypeRef, enum_names: &std::collections::HashSet<&str>) -> bool {
        match t {
            TypeRef::Named(name) => enum_names.contains(name.as_str()),
            TypeRef::Optional(inner) | TypeRef::Vec(inner) => type_ref_contains_enum(inner, enum_names),
            TypeRef::Map(k, v) => type_ref_contains_enum(k, enum_names) || type_ref_contains_enum(v, enum_names),
            _ => false,
        }
    }

    api.types
        .iter()
        .filter(|t| !t.is_trait && !t.is_opaque)
        .flat_map(|t| t.fields.iter())
        .any(|f| type_ref_contains_enum(&f.ty, &enum_names))
        || api.functions.iter().any(|f| {
            f.params.iter().any(|p| type_ref_contains_enum(&p.ty, &enum_names))
                || type_ref_contains_enum(&f.return_type, &enum_names)
        })
}

#[allow(dead_code)]
fn api_has_json_field(api: &ApiSurface) -> bool {
    api.types
        .iter()
        .flat_map(|t| t.fields.iter())
        .any(|f| type_has_json(&f.ty))
        || api
            .functions
            .iter()
            .any(|f| f.params.iter().any(|p| type_has_json(&p.ty)) || type_has_json(&f.return_type))
}

pub(crate) fn emit_cargo_toml(
    rust_dir: &str,
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    source_crate_name: &str,
) -> GeneratedFile {
    let crate_name = config.name.as_str();
    let version = &api_version(config);
    let frb_version = crate::naming::dart_frb_version(config);
    let core_crate_dir = config.core_crate_for_language(alef_core::config::extras::Language::Dart);
    let dart_override = config.dart.as_ref().and_then(|c| c.core_crate_override.as_deref());
    // Cargo dep KEY: when an override is set, use it as-is; otherwise preserve
    // the historical behaviour (`source_crate_name`, the Rust-ident form of
    // the umbrella crate name) so existing configs produce identical output.
    let core_dep_key: String = match dart_override {
        Some(name) => name.to_string(),
        None => source_crate_name.to_string(),
    };
    let same_as_workspace = dart_override.is_none() && core_crate_dir == *crate_name && config.workspace_root.is_none();
    let core_path = if same_as_workspace {
        "../../..".to_string()
    } else {
        format!("../../../crates/{core_crate_dir}")
    };

    let features = config.features_for_language(alef_core::config::extras::Language::Dart);
    let features_block = if features.is_empty() {
        String::new()
    } else {
        let list = features
            .iter()
            .map(|f| format!("\"{f}\""))
            .collect::<Vec<_>>()
            .join(", ");
        format!(", features = [{list}]")
    };

    // When the Rust ident form of the umbrella crate name (`core_dep_key`,
    // e.g. `liter_llm`) differs from the actual cargo package name in the
    // umbrella Cargo.toml (`crate_name`, e.g. `liter-llm`), cargo will not
    // resolve the path dependency unless we add an explicit `package = "..."`
    // rename. Use `crate_name` (the [[crates]] `name` field, which is the
    // cargo package name) rather than `core_crate_dir` (the directory name)
    // because the two can differ — e.g. `[[crates]] name = "html-to-markdown-rs"`
    // with sources under `crates/html-to-markdown/` where the package on disk
    // is `html-to-markdown-rs` but the directory is `html-to-markdown`.
    let package_rename_block = if dart_override.is_none() && core_dep_key != crate_name {
        format!(", package = \"{crate_name}\"")
    } else {
        String::new()
    };

    // Trait bridge impl methods use tokio::runtime::Handle::current().block_on(...) and
    // async-trait for async trait impls. Add these only when trait bridges are configured.
    // Note: anyhow is NOT included — bridge impls use source_crate::Result directly.
    let has_trait_bridges = config.trait_bridges.iter().any(|b| {
        !b.exclude_languages.iter().any(|l| l == "dart")
            && api.types.iter().any(|t| t.name == b.trait_name && t.is_trait)
    });
    let trait_bridge_deps = if has_trait_bridges {
        "tokio = { version = \"1\", features = [\"rt\"] }\nasync-trait = \"0.1\"\n"
    } else {
        ""
    };

    // Merge [crate.extra_dependencies] from alef.toml — required for multi-crate
    // workspaces where the bindings codegen emits qualified paths from sibling
    // crates (e.g. mylib_extra::QueryOnlyConfig). The umbrella crate is
    // already listed above; these are the additional sibling crates.
    let workspace_extra = config.extra_deps_for_language(alef_core::config::extras::Language::Dart);
    let mut workspace_dep_lines: Vec<String> = workspace_extra
        .iter()
        .map(|(name, value)| {
            if let Some(s) = value.as_str() {
                format!("{name} = \"{s}\"")
            } else {
                format!("{name} = {value}")
            }
        })
        .collect();
    workspace_dep_lines.sort();
    let workspace_deps_block = if workspace_dep_lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", workspace_dep_lines.join("\n"))
    };
    // serde_json is required when the generated From<SourceT> impls use
    // serde_json::to_string() to convert Json-typed fields (serde_json::Value,
    // ProcessResult, InternalDocument, etc.) OR enum-typed fields to String for the
    // FRB-friendly mirror. Detect by scanning the API surface for TypeRef::Json or
    // any Named field that resolves to an enum (D6 fix).
    let needs_serde_json = api_has_json_or_enum_field(api);
    let serde_json_dep = if needs_serde_json { "serde_json = \"1\"\n" } else { "" };
    // The dart streaming-adapter codegen emits `use futures_util::StreamExt;` and
    // calls `stream.next().await`, so add futures-util whenever the API has any
    // streaming adapters configured for dart.
    let has_streaming = config
        .adapters
        .iter()
        .any(|a| matches!(a.pattern, alef_core::config::extras::AdapterPattern::Streaming));
    let futures_util_dep = if has_streaming { "futures-util = \"0.3\"\n" } else { "" };
    let extra_deps = format!("{serde_json_dep}{futures_util_dep}{trait_bridge_deps}{workspace_deps_block}");

    let license = config
        .scaffold
        .as_ref()
        .and_then(|s| s.license.as_deref())
        .unwrap_or("MIT");

    // Build the cargo-machete ignored list: the umbrella crate plus every sibling
    // crate from [crate.extra_dependencies]. flutter_rust_bridge resolves types
    // across all of them, but the generated Rust wrapper only `use`s a subset —
    // cargo-machete would otherwise flag the rest.
    let mut machete_ignored: Vec<String> = std::iter::once(core_dep_key.clone())
        .chain(workspace_extra.keys().cloned())
        .collect();
    machete_ignored.sort();
    machete_ignored.dedup();
    let machete_ignored_list = machete_ignored
        .iter()
        .map(|n| format!("\"{n}\""))
        .collect::<Vec<_>>()
        .join(", ");

    // Per-target dependency overrides: if configured, emit the base core dep
    // gated on `cfg(not(<overrides>))` and an override block per cfg. The base
    // `flutter_rust_bridge` + extras stay in `[dependencies]` since they don't
    // change per target.
    let target_overrides = config
        .dart
        .as_ref()
        .map(|c| c.target_dep_overrides.as_slice())
        .unwrap_or(&[]);
    let (core_dep_line, target_override_blocks) = if target_overrides.is_empty() {
        (
            format!("{core_dep_key} = {{ path = \"{core_path}\"{package_rename_block}{features_block} }}\n"),
            String::new(),
        )
    } else {
        let neg_cfg = if target_overrides.len() == 1 {
            target_overrides[0].cfg.clone()
        } else {
            let any = target_overrides
                .iter()
                .map(|o| o.cfg.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            format!("any({any})")
        };
        let mut blocks = format!(
            "[target.'cfg(not({neg_cfg}))'.dependencies]\n{core_dep_key} = {{ path = \"{core_path}\"{package_rename_block}{features_block} }}\n\n"
        );
        for override_entry in target_overrides {
            let feat_list = override_entry
                .features
                .iter()
                .map(|f| format!("\"{f}\""))
                .collect::<Vec<_>>()
                .join(", ");
            let feats_block = if feat_list.is_empty() {
                String::new()
            } else {
                format!(", features = [{feat_list}]")
            };
            let default_block = if override_entry.default_features {
                String::new()
            } else {
                ", default-features = false".to_string()
            };
            blocks.push_str(&format!(
                "[target.'cfg({cfg})'.dependencies]\n{core_dep_key} = {{ path = \"{core_path}\"{package_rename_block}{default_block}{feats_block} }}\n\n",
                cfg = override_entry.cfg,
            ));
        }
        (String::new(), blocks)
    };

    let content = format!(
        r#"[package]
name = "{crate_name}-dart"
version = "{version}"
edition = "2024"
license = "{license}"

[package.metadata.cargo-machete]
# Umbrella + sibling crates are pulled in so flutter_rust_bridge can resolve
# every referenced type, but the generated Rust wrapper only `use`s a subset.
ignored = [{machete_ignored_list}]

[lib]
crate-type = ["cdylib", "staticlib"]

[dependencies]
{core_dep_line}flutter_rust_bridge = "{frb_version}"
{extra_deps}
{target_override_blocks}[lints.rust]
# flutter_rust_bridge uses #[cfg(frb_expand)] internally during macro expansion.
# Declare it as a known cfg so rustc does not emit unexpected_cfgs warnings.
unexpected_cfgs = {{ level = "warn", check-cfg = ['cfg(frb_expand)'] }}"#
    );

    GeneratedFile {
        path: PathBuf::from(rust_dir).join("Cargo.toml"),
        content,
        generated_header: false,
    }
}

pub(crate) fn emit_build_rs(rust_dir: &str) -> GeneratedFile {
    // Invoke `flutter_rust_bridge_codegen generate` at `cargo build` time so that
    // `src/frb_generated.rs` is always present before rustc tries to compile
    // `mod frb_generated;` in lib.rs. The invocation is conditional: when the
    // tool is not on PATH the build emits a cargo warning and proceeds against
    // the committed generated sources. This keeps `cargo check --workspace` and
    // `cargo build` working in CI environments and downstream projects that do
    // not have FRB installed.
    let content = r#"fn main() {
    // Re-run whenever any Rust source changes.
    println!("cargo:rerun-if-changed=src");

    // Optional FRB codegen: regenerate flutter_rust_bridge artifacts when the
    // tool is on PATH. Missing tool is not fatal — committed generated sources
    // are checked in, and CI environments without FRB still build cleanly.
    match std::process::Command::new("flutter_rust_bridge_codegen")
        .args(["generate", "--config-file", "flutter_rust_bridge.yaml"])
        .status()
    {
        Ok(status) if status.success() => {
            // FRB v2.12+ emits `use` lists in an order rustfmt 2024 edition rewrites
            // (e.g. `{transform_result_dco, Lifetimeable, Lockable}` →
            // `{Lifetimeable, Lockable, transform_result_dco}`). Run rustfmt against
            // the generated file so committed output is fmt-clean and `cargo fmt --check`
            // stays green in CI.
            match std::process::Command::new("rustfmt")
                .args(["--edition", "2024", "src/frb_generated.rs"])
                .status()
            {
                Ok(s) if s.success() => {}
                Ok(s) => println!("cargo:warning=rustfmt on src/frb_generated.rs exited {s}"),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    println!(
                        "cargo:warning=rustfmt not on PATH — skipping post-FRB format. Install rustfmt via rustup to keep generated bridge sources fmt-clean."
                    );
                }
                Err(err) => println!("cargo:warning=failed to spawn rustfmt: {err}"),
            }
        }
        Ok(status) => panic!("flutter_rust_bridge_codegen generate failed (exit code: {status})"),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            println!(
                "cargo:warning=flutter_rust_bridge_codegen not on PATH — skipping codegen. Install via `dart pub global activate flutter_rust_bridge_codegen` to regenerate FRB artifacts at build time."
            );
        }
        Err(err) => panic!("failed to spawn flutter_rust_bridge_codegen: {err}"),
    }
}
"#
    .to_string();
    GeneratedFile {
        path: PathBuf::from(rust_dir).join("build.rs"),
        content,
        generated_header: false,
    }
}

pub(crate) fn emit_frb_yaml(rust_dir: &str, module_name: &str) -> GeneratedFile {
    // FRB v2 schema: `rust_root` points at the Rust crate dir, `rust_input` at the
    // module path(s) to scan for `pub` items (the alef-generated crate places its
    // entire surface at the crate root `lib.rs`), and `dart_output` at the bindings
    // directory. `rust_input` is required by the FRB CLI even in v2 — omitting it
    // causes `flutter_rust_bridge_codegen generate` to panic with
    // "Please provide `rust_input`".
    // `add_mod_to_lib: false` prevents FRB codegen from prepending its own
    // `mod frb_generated;` at line 1 of lib.rs — alef already emits it in the
    // correct position (after crate-level #![allow] attrs) to avoid E0753.
    let content = format!(
        "rust_root: .\nrust_input: crate\ndart_output: ../lib/src/{module_name}_bridge_generated\nadd_mod_to_lib: false\n"
    );
    GeneratedFile {
        path: PathBuf::from(rust_dir).join("flutter_rust_bridge.yaml"),
        content,
        generated_header: false,
    }
}

fn api_version(config: &ResolvedCrateConfig) -> String {
    // Use the resolved version from Cargo.toml if available, otherwise fall back to "0.1.0"
    // as a safe default (the real version is resolved from Cargo.toml at publish time).
    config.resolved_version().unwrap_or_else(|| "0.1.0".to_string())
}
