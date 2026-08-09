use crate::backends::dart::template_env;
use crate::codegen::cfg as shared_cfg;
use crate::core::backend::GeneratedFile;
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, TypeRef};
use crate::core::template_versions as tv;
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
    excluded_type_paths: &std::collections::BTreeMap<String, String>,
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
        .trait_bridges_for(Language::Dart)
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
    // Consumers keep this crate in their own Cargo workspace, so a literal `version =
    // "…"` here silently falls behind on the next workspace-wide bump. Mirror every
    // other binding-crate emitter (ffi.rs/php.rs/ruby.rs/node.rs/python.rs) and inherit
    // `[workspace.package] version` when the root Cargo.toml declares one; a standalone
    // (non-workspace) consumer still gets the literal. ~keep
    //
    // ~keep Membership-aware, like ruby.rs and ffi.rs: this crate is routinely named in the
    // root's `[workspace] exclude` so flutter_rust_bridge builds it with its own resolver
    // rather than the workspace's. `detect_workspace_inheritance` only asks whether a
    // workspace exists, so for an excluded crate it emitted `version.workspace = true` into a
    // manifest that can never resolve it -- cargo then fails to parse the manifest outright
    // ("error inheriting `version` from workspace root manifest") and takes
    // `flutter_rust_bridge_codegen` down with it, which is exactly the failure
    // `detect_workspace_inheritance_for_crate` documents and exists to prevent.
    let dart_crate_dir = config.package_dir(crate::core::config::extras::Language::Dart);
    let ws = crate::scaffold::detect_workspace_inheritance_for_crate(
        config.workspace_root.as_deref(),
        &format!("{dart_crate_dir}/rust"),
    );
    let version_line = if ws.version {
        "version.workspace = true".to_string()
    } else {
        format!("version = \"{version}\"")
    };
    let frb_version = crate::backends::dart::naming::dart_frb_version(config);
    let core_crate_dir = config.core_crate_for_language(crate::core::config::extras::Language::Dart);
    let dart_override = config.dart.as_ref().and_then(|c| c.core_crate_override.as_deref());
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

    // `[crates.dart].excluded_default_features` must also drop a name from this explicit
    // `features = [...]` line on the core dependency, not just the wrapper's own `default =
    // [...]` array below -- forwarding an excluded name here unions it straight back into the
    // core crate via Cargo's feature unification, defeating a `target_dep_overrides` entry that
    // turned it off for a specific cfg target. This is the same defect
    // `RubyConfig::excluded_default_features` fixed for the Magnus crate, generalized in
    // `scaffold::core_dep_features_excluding`. ~keep
    let excluded_default_features: std::collections::HashSet<&str> = config
        .dart
        .as_ref()
        .map(|c| c.excluded_default_features.iter().map(String::as_str).collect())
        .unwrap_or_default();
    let features_block = crate::scaffold::core_dep_features_excluding(
        config,
        crate::core::config::extras::Language::Dart,
        &excluded_default_features,
    );

    let package_rename_block = if dart_override.is_none() && core_dep_key != crate_name {
        format!(", package = \"{crate_name}\"")
    } else {
        String::new()
    };

    let has_trait_bridges = config.trait_bridges_for(Language::Dart).any(|b| {
        !b.exclude_languages.iter().any(|l| l == "dart")
            && api.types.iter().any(|t| t.name == b.trait_name && t.is_trait)
    });
    let trait_bridge_deps = if has_trait_bridges {
        format!("async-trait = \"{}\"\n", tv::cargo::ASYNC_TRAIT)
    } else {
        String::new()
    };

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
    crate::scaffold::sort_dependency_lines(&mut workspace_dep_lines);
    let has_trait_bridge_excluded_carrier = api_has_trait_bridge_excluded_carrier(api, config);
    let needs_serde_json = api_has_json_or_enum_field(api) || has_trait_bridge_excluded_carrier;
    let serde_json_dep = if needs_serde_json {
        format!("serde_json = \"{}\"\n", tv::cargo::SERDE_JSON)
    } else {
        String::new()
    };
    let needs_serde_derive = has_trait_bridge_excluded_carrier;
    let serde_dep = if needs_serde_derive {
        format!(
            "serde = {{ version = \"{}\", features = [\"derive\"] }}\n",
            tv::cargo::SERDE
        )
    } else {
        String::new()
    };
    let needs_ahash = api_has_ahash_param(api);
    let ahash_dep = if needs_ahash {
        format!("ahash = \"{}\"\n", tv::cargo::AHASH)
    } else {
        String::new()
    };
    let has_streaming = config
        .adapters
        .iter()
        .any(|a| matches!(a.pattern, crate::core::config::extras::AdapterPattern::Streaming));
    let futures_util_dep = if has_streaming {
        format!("futures-util = \"{}\"\n", tv::cargo::FUTURES_UTIL)
    } else {
        String::new()
    };
    // `tokio::sync::Mutex<Option<…>>` for thread-safe handoff between `#[frb(sync)]`
    let has_services = !api.services.is_empty();
    let tokio_dep = if has_streaming || has_trait_bridges || has_services {
        format!(
            "tokio = {{ version = \"{}\", features = [\"rt-multi-thread\", \"sync\"] }}\n",
            tv::cargo::TOKIO
        )
    } else {
        String::new()
    };
    let target_overrides = config
        .dart
        .as_ref()
        .map(|c| c.target_dep_overrides.as_slice())
        .unwrap_or(&[]);

    // lives in `[target.'cfg(...)'.dependencies]` blocks instead).
    let frb_line = format!("flutter_rust_bridge = \"={frb_version}\"");
    let mut dep_lines: Vec<String> = Vec::new();
    if target_overrides.is_empty() {
        dep_lines.push(format!(
            "{core_dep_key} = {{ path = \"{core_path}\"{package_rename_block}{features_block} }}"
        ));
    }
    dep_lines.push(frb_line);
    if !config.components.is_empty() {
        let alef_version = env!("CARGO_PKG_VERSION");
        for (name, line) in [
            ("alef-component-abi", format!("alef-component-abi = \"{alef_version}\"")),
            (
                "alef-component-runtime",
                format!("alef-component-runtime = \"{alef_version}\""),
            ),
            ("directories", "directories = \"6\"".to_string()),
        ] {
            if !workspace_extra.contains_key(name) {
                dep_lines.push(line);
            }
        }
        if !needs_serde_json && !workspace_extra.contains_key("serde_json") {
            dep_lines.push("serde_json = \"1\"".to_string());
        }
    }
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
    if has_trait_bridges {
        dep_lines.push(format!(
            "tracing = \"{}\"",
            crate::core::template_versions::cargo::TRACING
        ));
    }
    dep_lines.extend(workspace_dep_lines);
    crate::scaffold::sort_dependency_lines(&mut dep_lines);
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

    let mut machete_ignored: Vec<String> = std::iter::once(core_dep_key.clone())
        .chain(workspace_extra.keys().cloned())
        .collect();
    if api_has_ahash_param(api) {
        machete_ignored.push("ahash".to_string());
    }
    if has_trait_bridges {
        machete_ignored.push("async-trait".to_string());
        machete_ignored.push("tracing".to_string());
    }
    machete_ignored.sort();
    machete_ignored.dedup();
    let machete_ignored_list = machete_ignored
        .iter()
        .map(|n| format!("\"{n}\""))
        .collect::<Vec<_>>()
        .join(", ");

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
        let default_cfg = format!("not({neg_cfg})");
        let mut entries: Vec<(String, String)> = vec![(
            default_cfg.clone(),
            template_env::render(
                "rust_cargo_target_dependency.rs.jinja",
                minijinja::context! {
                    cfg => default_cfg.as_str(),
                    core_dep_key => core_dep_key.as_str(),
                    core_path => core_path.as_str(),
                    package_rename_block => package_rename_block.as_str(),
                    default_block => "",
                    features_block => features_block.as_str(),
                },
            ),
        )];
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
            entries.push((
                override_entry.cfg.clone(),
                template_env::render(
                    "rust_cargo_target_dependency.rs.jinja",
                    minijinja::context! {
                        cfg => override_entry.cfg.as_str(),
                        core_dep_key => core_dep_key.as_str(),
                        core_path => core_path.as_str(),
                        package_rename_block => package_rename_block.as_str(),
                        default_block => default_block.as_str(),
                        features_block => feats_block.as_str(),
                    },
                ),
            ));
        }
        // cargo-sort orders `[target.'cfg(...)'.dependencies]` tables
        // alphabetically by the raw cfg predicate string (plain byte-wise
        // comparison), so the default `not(...)` branch is not always first —
        // e.g. an `all(...)` override (macOS-Intel) sorts before it. ~keep
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        let blocks = entries.into_iter().map(|(_, text)| text).collect::<String>();
        (String::new(), blocks)
    };

    // `#[cfg(feature = "X")]` arms emitted by the codegen produce
    let cfg_features_table: String = {
        let mut features = shared_cfg::collect_cfg_features(api);
        // A config-only `excluded_default_features` name (gates no `#[cfg(feature = ...)]`) must
        // still get a forwarding entry below -- alef-task #371, regression in
        // `cargo_excluded_features_tests.rs`. ~keep
        features.extend(excluded_default_features.iter().map(|name| (*name).to_string()));
        if features.is_empty() {
            String::new()
        } else {
            // `[target.'cfg(...)'.dependencies]` block alone is insufficient
            let mut lines: Vec<String> = Vec::with_capacity(features.len() + 1);
            // `#[cfg(feature = "X")]` arms emitted by the codegen compile
            let default_list: Vec<String> = features
                .iter()
                .filter(|name| !excluded_default_features.contains(name.as_str()))
                .map(|name| format!("\"{name}\""))
                .collect();
            lines.push(format!("default = [{}]", default_list.join(", ")));
            for name in &features {
                lines.push(format!(r#"{name} = ["{core_dep_key}/{name}"]"#));
            }
            let passthrough_names: Vec<&str> = features.iter().map(String::as_str).collect();
            if let Some(line) =
                crate::scaffold::android_target_feature_line_for_dep(config, &core_dep_key, &passthrough_names)
            {
                lines.push(line);
            }
            format!("[features]\n{}\n", lines.join("\n"))
        }
    };

    // The [lints.rust] block keeps cfg(frb_expand) in the allow-list (FRB-internal
    // cfg used during macro expansion). Cargo allows only one `[lints.rust]` table
    // per manifest, so a configured `[crates.cargo_lints.rust]` entry becomes an
    // extra sibling line under this same hand-written header rather than a second
    // one -- see `CargoLintsConfig::extra_rust_lines`. ~keep
    let mut lints_rust_lines = vec![
        "# flutter_rust_bridge uses #[cfg(frb_expand)] internally during macro expansion.".to_string(),
        "unexpected_cfgs = { level = \"warn\", check-cfg = ['cfg(frb_expand)'] }".to_string(),
    ];
    lints_rust_lines.extend(config.cargo_lints.extra_rust_lines(&["unexpected_cfgs"]));
    let clippy_block = config.cargo_lints.clippy_block();
    let clippy_section = if clippy_block.is_empty() {
        String::new()
    } else {
        format!("\n\n{clippy_block}")
    };
    let lints_block = format!("[lints.rust]\n{}{clippy_section}", lints_rust_lines.join("\n"));

    let content = template_env::render(
        "rust_cargo_toml.rs.jinja",
        minijinja::context! {
            crate_name => crate_name,
            version_line => version_line.as_str(),
            license => license,
            machete_ignored_list => machete_ignored_list.as_str(),
            core_dep_line => core_dep_line.as_str(),
            frb_version => frb_version.as_str(),
            extra_deps => extra_deps.as_str(),
            target_override_blocks => target_override_blocks.as_str(),
            cfg_features_table => cfg_features_table.as_str(),
            lints_block => lints_block.as_str(),
        },
    );

    // `.toml` is on `generate::write::marker_comment_style`'s *ownership* list, where a missing
    // provenance marker is read as proof alef never authored the file: `write_files_report`
    // refuses the write, so the marker never lands, so it refuses forever. This manifest's
    // `[features]` table and `lib.rs`'s `#[cfg(feature = "...")]` gates are derived from one
    // `collect_cfg_features` call on one surface and cannot disagree in memory — they disagree on
    // disk when only the self-marking `lib.rs` is written, which is how a newly cfg-gated item
    // reaches the generated crate as `unexpected_cfg`. Stamping keeps both files on the same
    // rail. ~keep
    GeneratedFile {
        path: PathBuf::from(format!("{rust_dir}/Cargo.toml")),
        content,
        generated_header: true,
    }
}

fn api_version(config: &ResolvedCrateConfig) -> String {
    config.resolved_version().unwrap_or_else(|| "0.1.0".to_string())
}

#[cfg(test)]
mod feature_cfg_tests;
