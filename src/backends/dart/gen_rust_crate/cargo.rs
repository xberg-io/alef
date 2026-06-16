use crate::backends::dart::template_env;
use crate::codegen::cfg as shared_cfg;
use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, TypeRef};
use std::path::PathBuf;

/// Returns true when any function parameter has `map_is_ahash = true`, meaning
/// the generated bridge fn references `ahash::AHashMap` in a pre-call binding.
fn api_has_ahash_param(api: &ApiSurface) -> bool {
    api.functions.iter().any(|f| f.params.iter().any(|p| p.map_is_ahash))
}

fn type_has_json(t: &TypeRef) -> bool {
    match t {
        TypeRef::Json => true,
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => type_has_json(inner),
        TypeRef::Map(k, v) => type_has_json(k) || type_has_json(v),
        _ => false,
    }
}

fn type_references_excluded_named(
    t: &TypeRef,
    excluded_type_paths: &std::collections::HashMap<String, String>,
) -> bool {
    match t {
        TypeRef::Named(name) => excluded_type_paths.contains_key(name),
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => type_references_excluded_named(inner, excluded_type_paths),
        TypeRef::Map(k, v) => {
            type_references_excluded_named(k, excluded_type_paths)
                || type_references_excluded_named(v, excluded_type_paths)
        }
        _ => false,
    }
}

fn api_has_trait_bridge_excluded_carrier(api: &ApiSurface, config: &ResolvedCrateConfig) -> bool {
    config
        .trait_bridges
        .iter()
        .filter(|cfg| !cfg.exclude_languages.iter().any(|l| l == "dart"))
        .filter_map(|cfg| api.types.iter().find(|t| t.name == cfg.trait_name && t.is_trait))
        .flat_map(|trait_def| trait_def.methods.iter())
        .filter(|m| m.trait_source.is_none())
        .any(|m| {
            type_references_excluded_named(&m.return_type, &api.excluded_type_paths)
                || m.params
                    .iter()
                    .any(|p| type_references_excluded_named(&p.ty, &api.excluded_type_paths))
        })
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
    let frb_version = crate::backends::dart::naming::dart_frb_version(config);
    let core_crate_dir = config.core_crate_for_language(crate::core::config::extras::Language::Dart);
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

    let features = config.features_for_language(crate::core::config::extras::Language::Dart);
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
    // e.g. `sample_llm`) differs from the actual cargo package name in the
    // umbrella Cargo.toml (`crate_name`, e.g. `sample-llm`), cargo will not
    // resolve the path dependency unless we add an explicit `package = "..."`
    // rename. Use `crate_name` (the [[crates]] `name` field, which is the
    // cargo package name) rather than `core_crate_dir` (the directory name)
    // because the two can differ — e.g. `[[crates]] name = "sample-markdown-rs"`
    // with sources under `crates/sample-markdown/` where the package on disk
    // is `sample-markdown-rs` but the directory is `sample-markdown`.
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
        "async-trait = \"0.1\"\n"
    } else {
        ""
    };

    // Merge [crate.extra_dependencies] from alef.toml — required for multi-crate
    // workspaces where the bindings codegen emits qualified paths from sibling
    // crates (e.g. mylib_extra::QueryOnlyConfig). The umbrella crate is
    // already listed above; these are the additional sibling crates.
    let workspace_extra = config.extra_deps_for_language(crate::core::config::extras::Language::Dart);
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
    // serde_json is required when generated conversions serialize JSON fields,
    // enum fields, or excluded trait-bridge carrier values.
    let has_trait_bridge_excluded_carrier = api_has_trait_bridge_excluded_carrier(api, config);
    let needs_serde_json = api_has_json_or_enum_field(api) || has_trait_bridge_excluded_carrier;
    let serde_json_dep = if needs_serde_json { "serde_json = \"1\"\n" } else { "" };
    // Excluded-type carrier derives need serde as a direct dependency.
    let needs_serde_derive = has_trait_bridge_excluded_carrier;
    let serde_dep = if needs_serde_derive {
        "serde = { version = \"1\", features = [\"derive\"] }\n"
    } else {
        ""
    };
    // ahash is needed when any function takes an AHashMap<Cow, _> param — the generated
    // bridge fn emits a `let __<name>_ahash: ahash::AHashMap<...>` pre-call binding.
    let needs_ahash = api_has_ahash_param(api);
    let ahash_dep = if needs_ahash { "ahash = \"0.8\"\n" } else { "" };
    // The dart streaming-adapter codegen emits `use futures_util::StreamExt;` and
    // calls `stream.next().await`, so add futures-util whenever the API has any
    // streaming adapters configured for dart.
    let has_streaming = config
        .adapters
        .iter()
        .any(|a| matches!(a.pattern, crate::core::config::extras::AdapterPattern::Streaming));
    let futures_util_dep = if has_streaming { "futures-util = \"0.3\"\n" } else { "" };
    // Trait-bridge impls use `tokio::runtime::Handle::current().block_on(...)`
    // and the streaming codegen installs a shared multi-thread runtime via
    // `OnceLock`. The service-API codegen wraps the inner service owner in a
    // `tokio::sync::Mutex<Option<…>>` for thread-safe handoff between `#[frb(sync)]`
    // registration calls and the async `run()` entrypoint. Merge all three needs
    // into a single declaration: `rt-multi-thread` (transitively includes `rt`)
    // plus `sync` (for `tokio::sync::Mutex`).
    let has_services = !api.services.is_empty();
    let tokio_dep = if has_streaming || has_trait_bridges || has_services {
        "tokio = { version = \"1\", features = [\"rt-multi-thread\", \"sync\"] }\n"
    } else {
        ""
    };
    let target_overrides = config
        .dart
        .as_ref()
        .map(|c| c.target_dep_overrides.as_slice())
        .unwrap_or(&[]);

    // Collect every line that belongs in `[dependencies]` and sort them alphabetically
    // by package name so the generated Cargo.toml is idempotent under cargo-sort.
    //
    // The conditional `*_dep` strings above are either empty or shaped as
    // `"<name> = <value>\n"`; strip the trailing newline and parse the package name
    // from the substring before the first `=`. Core + flutter_rust_bridge are
    // unconditional (core is omitted when per-target overrides apply, since it then
    // lives in `[target.'cfg(...)'.dependencies]` blocks instead).
    let frb_line = format!("flutter_rust_bridge = \"={frb_version}\"");
    let mut dep_lines: Vec<String> = Vec::new();
    if target_overrides.is_empty() {
        dep_lines.push(format!(
            "{core_dep_key} = {{ path = \"{core_path}\"{package_rename_block}{features_block} }}"
        ));
    }
    dep_lines.push(frb_line);
    for dep in [
        ahash_dep,
        serde_dep,
        serde_json_dep,
        futures_util_dep,
        tokio_dep,
        trait_bridge_deps,
    ] {
        let trimmed = dep.trim_end_matches('\n');
        if !trimmed.is_empty() {
            dep_lines.push(trimmed.to_string());
        }
    }
    dep_lines.extend(workspace_dep_lines);
    dep_lines.sort_by(|a, b| {
        let key = |line: &str| line.split('=').next().unwrap_or("").trim().to_string();
        key(a).cmp(&key(b))
    });
    let extra_deps = if dep_lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", dep_lines.join("\n"))
    };

    let license = config
        .scaffold
        .as_ref()
        .and_then(|s| s.license.as_deref())
        .unwrap_or("MIT");

    // Build the cargo-machete ignored list: the umbrella crate plus every sibling
    // crate from [crate.extra_dependencies]. flutter_rust_bridge resolves types
    // across all of them, but the generated Rust wrapper only `use`s a subset —
    // cargo-machete would otherwise flag the rest. `ahash` is added when any
    // function parameter uses AHashMap<Cow, _>, but the bridge never directly uses
    // ahash—it's used only in the Rust core for type field marshalling.
    let mut machete_ignored: Vec<String> = std::iter::once(core_dep_key.clone())
        .chain(workspace_extra.keys().cloned())
        .collect();
    if api_has_ahash_param(api) {
        machete_ignored.push("ahash".to_string());
    }
    machete_ignored.sort();
    machete_ignored.dedup();
    let machete_ignored_list = machete_ignored
        .iter()
        .map(|n| format!("\"{n}\""))
        .collect::<Vec<_>>()
        .join(", ");

    // Per-target dependency overrides: if configured, emit the base core dep
    // gated on `cfg(not(<overrides>))` and an override block per cfg.
    let (core_dep_line, target_override_blocks) = if target_overrides.is_empty() {
        (String::new(), String::new())
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
        let mut blocks = template_env::render(
            "rust_cargo_target_dependency.rs.jinja",
            minijinja::context! {
                cfg => format!("not({neg_cfg})"),
                core_dep_key => core_dep_key.as_str(),
                core_path => core_path.as_str(),
                package_rename_block => package_rename_block.as_str(),
                default_block => "",
                features_block => features_block.as_str(),
            },
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
            blocks.push_str(&template_env::render(
                "rust_cargo_target_dependency.rs.jinja",
                minijinja::context! {
                    cfg => override_entry.cfg.as_str(),
                    core_dep_key => core_dep_key.as_str(),
                    core_path => core_path.as_str(),
                    package_rename_block => package_rename_block.as_str(),
                    default_block => default_block.as_str(),
                    features_block => feats_block.as_str(),
                },
            ));
        }
        (String::new(), blocks)
    };

    // Collect every feature name referenced by a cfg attribute on any type, field,
    // enum variant, or function in the API surface and emit a forwarding `[features]`
    // table so the binding crate can re-export them to the core dep. Without this,
    // `#[cfg(feature = "X")]` arms emitted by the codegen produce
    // `error: unexpected cfg condition value: X` because the binding crate's
    // `Cargo.toml` does not declare that feature.
    let cfg_features_table: String = {
        let features = shared_cfg::collect_cfg_features(api);
        if features.is_empty() {
            String::new()
        } else {
            let mut lines: Vec<String> = Vec::with_capacity(features.len() + 1);
            // Enable all cfg-forwarded features by default so that
            // `#[cfg(feature = "X")]` arms emitted by the codegen compile
            // without requiring the caller to explicitly activate them.
            // Mirrors the swift backend behaviour (see cargo.rs there).
            let default_list: Vec<String> = features.iter().map(|name| format!("\"{name}\"")).collect();
            lines.push(format!("default = [{}]", default_list.join(", ")));
            for name in &features {
                lines.push(format!(r#"{name} = ["{core_dep_key}/{name}"]"#));
            }
            format!("[features]\n{}\n", lines.join("\n"))
        }
    };

    let content = template_env::render(
        "rust_cargo_toml.rs.jinja",
        minijinja::context! {
            crate_name => crate_name,
            version => version.as_str(),
            license => license,
            machete_ignored_list => machete_ignored_list.as_str(),
            core_dep_line => core_dep_line.as_str(),
            frb_version => frb_version.as_str(),
            extra_deps => extra_deps.as_str(),
            target_override_blocks => target_override_blocks.as_str(),
            cfg_features_table => cfg_features_table.as_str(),
        },
    );

    GeneratedFile {
        path: PathBuf::from(format!("{rust_dir}/Cargo.toml")),
        content,
        generated_header: false,
    }
}

pub(crate) fn emit_build_rs(rust_dir: &str, package_name: &str, module_name: &str, stem: &str) -> GeneratedFile {
    // Invoke `flutter_rust_bridge_codegen generate` at `cargo build` time so that
    // `src/frb_generated.rs` is always present before rustc tries to compile
    // `mod frb_generated;` in lib.rs. The invocation is conditional: when the
    // tool is not on PATH the build emits a cargo warning and proceeds against
    // the committed generated sources. This keeps `cargo check --workspace` and
    // `cargo build` working in CI environments and downstream projects that do
    // not have FRB installed.
    //
    // After codegen, the generated `frb_generated.dart` is patched so the
    // published package can locate its native library from its own installed
    // location (see `patch_published_loader`) instead of flutter_rust_bridge's
    // build-tree-relative default `ioDirectory`.
    let loader_patch = render_loader_patch_fn(package_name, module_name, stem);
    let content = template_env::render(
        "rust_build_rs.rs.jinja",
        minijinja::context! {
            loader_patch => loader_patch.as_str(),
        },
    );
    GeneratedFile {
        path: PathBuf::from(format!("{rust_dir}/build.rs")),
        content,
        generated_header: false,
    }
}

/// Render the `patch_published_loader` Rust function embedded in the generated
/// dart bridge crate's `build.rs`.
///
/// flutter_rust_bridge's default loader uses a build-tree-relative `ioDirectory`
/// (e.g. `rust/target/release/`) resolved against the *consumer's* current
/// working directory — a path that is not shipped in the published pub tarball.
/// Consuming the package from pub.dev therefore fails to find the library and
/// falls back to opening a relative framework path (rejected by hardened
/// runtimes). This patcher injects a loader that resolves the prebuilt library
/// from the package's own installed location (`lib/src/<module>_bridge_generated/`,
/// resolved via `Isolate.resolvePackageUri`) as an absolute path, falling back
/// to flutter_rust_bridge's default loader when that library is absent (e.g.
/// local development builds). The patch is idempotent (keyed off a marker) and a
/// no-op when the FRB entrypoint signature is absent.
fn render_loader_patch_fn(package_name: &str, module_name: &str, stem: &str) -> String {
    // The Dart loader-helper method + patched `init` prologue, computed once and
    // embedded as a raw literal in the generated build.rs. The original FRB
    // `init` prologue is matched verbatim and replaced.
    let dart_replacement = dart_init_prologue_replacement(package_name, module_name, stem);
    template_env::render(
        "rust_loader_patch_fn.rs.jinja",
        minijinja::context! {
            module_name => module_name,
            dart_replacement => dart_replacement.as_str(),
        },
    )
}

/// Build the patched `RustLib.init` prologue Dart source: the loader helper
/// method followed by the original `init` signature with a resolution line that
/// prefers the package-relative library.
///
/// Kept in sync with the FRB 2.x `RustLib.init` signature. Published pub.dev
/// packages stage natives under `lib/src/native/<rid>/` (e.g. `macos-arm64`,
/// `linux-x64`). For local FRB-dev builds the dylib is emitted into
/// `lib/src/{module}_bridge_generated/` and is searched as a fallback.
fn dart_init_prologue_replacement(package_name: &str, module_name: &str, stem: &str) -> String {
    template_env::render(
        "dart_init_prologue_replacement.jinja",
        minijinja::context! {
            package_name => package_name,
            module_name => module_name,
            stem => stem,
        },
    )
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
    let content = template_env::render(
        "flutter_rust_bridge_yaml.jinja",
        minijinja::context! {
            module_name => module_name,
        },
    );
    GeneratedFile {
        path: PathBuf::from(format!("{rust_dir}/flutter_rust_bridge.yaml")),
        content,
        generated_header: false,
    }
}

fn api_version(config: &ResolvedCrateConfig) -> String {
    // Use the resolved version from Cargo.toml if available, otherwise fall back to "0.1.0"
    // as a safe default (the real version is resolved from Cargo.toml at publish time).
    config.resolved_version().unwrap_or_else(|| "0.1.0".to_string())
}

#[cfg(test)]
mod feature_cfg_tests {
    use super::*;
    use crate::core::ir::{EnumDef, EnumVariant};

    fn make_unit_variant(name: &str, cfg: Option<&str>) -> EnumVariant {
        EnumVariant {
            name: name.to_string(),
            cfg: cfg.map(str::to_string),
            ..Default::default()
        }
    }

    /// When the API has cfg-gated enum variants the emitted Cargo.toml must declare
    /// a forwarding `[features]` block mapping each referenced feature to the core
    /// dep. This is Option B — the binding crate re-exports the feature rather than
    /// using a `[lints.rust]` check-cfg allow-list.
    #[test]
    fn cargo_toml_emits_forwarding_features_block_for_cfg_gated_variants() {
        use crate::core::config::ResolvedCrateConfig;
        use crate::core::ir::ApiSurface;

        let api = ApiSurface {
            enums: vec![EnumDef {
                name: "ImageOutputFormat".to_string(),
                variants: vec![
                    make_unit_variant("Native", None),
                    make_unit_variant("Heic", Some("feature = \"heic\"")),
                    make_unit_variant("Svg", Some("feature = \"svg\"")),
                ],
                ..Default::default()
            }],
            ..Default::default()
        };
        let config = ResolvedCrateConfig {
            name: "sample-lib".to_string(),
            ..Default::default()
        };
        let file = emit_cargo_toml("packages/dart/rust", &api, &config, "sample_lib");
        assert!(
            file.content.contains(r#"heic = ["sample_lib/heic"]"#),
            "Cargo.toml must forward `heic` feature to core dep; got:\n{}",
            file.content
        );
        assert!(
            file.content.contains(r#"svg = ["sample_lib/svg"]"#),
            "Cargo.toml must forward `svg` feature to core dep; got:\n{}",
            file.content
        );
        assert!(
            file.content.contains("[features]"),
            "Cargo.toml must contain a [features] section; got:\n{}",
            file.content
        );
        // The `default` feature must enable all cfg-forwarded features so
        // `#[cfg(feature = "X")]` arms compile without explicit activation.
        assert!(
            file.content.contains("default = ["),
            "Cargo.toml must contain a `default` feature list; got:\n{}",
            file.content
        );
        assert!(
            file.content.contains("\"heic\"") && file.content.contains("\"svg\""),
            "default feature list must include all cfg-forwarded features; got:\n{}",
            file.content
        );
        // frb_expand must still be declared (FRB-internal cfg key, not a feature).
        assert!(
            file.content.contains("'cfg(frb_expand)'"),
            "Cargo.toml must still include cfg(frb_expand); got:\n{}",
            file.content
        );
        // No feature values in check-cfg any more — forwarding replaces the allow-list.
        assert!(
            !file.content.contains("values("),
            "Cargo.toml must not contain check-cfg values() — forwarding replaces allow-list; got:\n{}",
            file.content
        );
        // Output must be valid TOML.
        toml::from_str::<toml::Value>(&file.content).expect("generated Cargo.toml must be valid TOML");
    }

    /// When no item has a cfg attribute the `[features]` block must be omitted.
    #[test]
    fn cargo_toml_omits_features_block_when_no_cfg_attrs() {
        use crate::core::config::ResolvedCrateConfig;
        use crate::core::ir::ApiSurface;

        let api = ApiSurface {
            enums: vec![EnumDef {
                name: "SimpleEnum".to_string(),
                variants: vec![make_unit_variant("A", None), make_unit_variant("B", None)],
                ..Default::default()
            }],
            ..Default::default()
        };
        let config = ResolvedCrateConfig {
            name: "sample-lib".to_string(),
            ..Default::default()
        };
        let file = emit_cargo_toml("packages/dart/rust", &api, &config, "sample_lib");
        assert!(
            file.content
                .contains("unexpected_cfgs = { level = \"warn\", check-cfg = ['cfg(frb_expand)'] }"),
            "Cargo.toml must use single-entry form when no cfg attrs; got:\n{}",
            file.content
        );
        assert!(
            !file.content.contains("[features]"),
            "Cargo.toml must not contain [features] block when no cfg attrs; got:\n{}",
            file.content
        );
        toml::from_str::<toml::Value>(&file.content).expect("generated Cargo.toml must be valid TOML");
    }

    /// cfg-gated types (not just variants) must also appear in the forwarding block.
    #[test]
    fn cargo_toml_forwarding_covers_type_level_cfg_attrs() {
        use crate::core::config::ResolvedCrateConfig;
        use crate::core::ir::{ApiSurface, TypeDef};

        let api = ApiSurface {
            types: vec![TypeDef {
                name: "PdfDoc".to_string(),
                rust_path: "mylib::PdfDoc".to_string(),
                cfg: Some(r#"feature = "pdf""#.to_string()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let config = ResolvedCrateConfig {
            name: "sample-lib".to_string(),
            ..Default::default()
        };
        let file = emit_cargo_toml("packages/dart/rust", &api, &config, "sample_lib");
        assert!(
            file.content.contains(r#"pdf = ["sample_lib/pdf"]"#),
            "Cargo.toml must forward `pdf` feature from type-level cfg; got:\n{}",
            file.content
        );
        toml::from_str::<toml::Value>(&file.content).expect("generated Cargo.toml must be valid TOML");
    }
}

#[cfg(test)]
mod build_rs_tests {
    use super::*;

    #[test]
    fn emitted_build_rs_is_valid_rust() {
        let file = emit_build_rs(
            "packages/dart/rust",
            "sample_router",
            "sample_router",
            "sample_router_dart",
        );
        syn::parse_file(&file.content).expect("generated build.rs must be valid Rust");
    }

    #[test]
    fn emitted_build_rs_patches_published_loader_after_codegen() {
        let file = emit_build_rs(
            "packages/dart/rust",
            "sample_router",
            "sample_router",
            "sample_router_dart",
        );
        // The patch runs only on a successful FRB codegen.
        assert!(
            file.content.contains("patch_published_loader();"),
            "build.rs must invoke the loader patch after codegen"
        );
        assert!(
            file.content.contains("fn patch_published_loader()"),
            "build.rs must define the loader patch"
        );
        // Targets the generated dart entrypoint by module name.
        assert!(
            file.content
                .contains(r#"../lib/src/sample_router_bridge_generated/frb_generated.dart"#),
            "build.rs must target the generated frb dart file"
        );
        // The injected Dart resolves the package-relative library.
        assert!(
            file.content
                .contains("Isolate.resolvePackageUri(Uri.parse('package:sample_router/sample_router.dart'))"),
            "build.rs replacement must resolve the package URI"
        );
        assert!(
            file.content
                .contains("externalLibrary ??= await _alefResolveExternalLibrary();"),
            "build.rs replacement must prefer the package-relative library"
        );
    }

    #[test]
    fn emitted_build_rs_runs_dart_format_after_patch() {
        let file = emit_build_rs(
            "packages/dart/rust",
            "sample_router",
            "sample_router",
            "sample_router_dart",
        );
        assert!(
            file.content.contains("Command::new(\"dart\")")
                && file.content.contains("\"format\"")
                && file.content.contains("FRB_GENERATED_DART"),
            "build.rs must run `dart format` on the patched frb_generated.dart"
        );
    }

    #[test]
    fn emitted_build_rs_handles_loader_patch_write_error() {
        let file = emit_build_rs(
            "packages/dart/rust",
            "sample_router",
            "sample_router",
            "sample_router_dart",
        );
        // The loader patch must report and return on write failures instead of continuing
        // to format a stale or partially-written generated Dart file.
        assert!(
            file.content
                .contains("if let Err(err) = std::fs::write(path, &patched)")
                && file
                    .content
                    .contains("cargo:warning=failed to write published-loader patch: {err}")
                && file.content.contains("return;"),
            "emitted build.rs must handle loader patch write errors"
        );
    }
}
