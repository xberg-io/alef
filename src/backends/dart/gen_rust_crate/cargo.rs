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
    let ws = crate::scaffold::detect_workspace_inheritance(config.workspace_root.as_deref());
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

    let has_trait_bridges = config.trait_bridges.iter().any(|b| {
        !b.exclude_languages.iter().any(|l| l == "dart")
            && api.types.iter().any(|t| t.name == b.trait_name && t.is_trait)
    });
    let trait_bridge_deps = if has_trait_bridges {
        "async-trait = \"0.1\"\n"
    } else {
        ""
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
    let serde_json_dep = if needs_serde_json { "serde_json = \"1\"\n" } else { "" };
    let needs_serde_derive = has_trait_bridge_excluded_carrier;
    let serde_dep = if needs_serde_derive {
        "serde = { version = \"1\", features = [\"derive\"] }\n"
    } else {
        ""
    };
    let needs_ahash = api_has_ahash_param(api);
    let ahash_dep = if needs_ahash { "ahash = \"0.8\"\n" } else { "" };
    let has_streaming = config
        .adapters
        .iter()
        .any(|a| matches!(a.pattern, crate::core::config::extras::AdapterPattern::Streaming));
    let futures_util_dep = if has_streaming { "futures-util = \"0.3\"\n" } else { "" };
    // `tokio::sync::Mutex<Option<…>>` for thread-safe handoff between `#[frb(sync)]`
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
        assert!(
            file.content.contains("'cfg(frb_expand)'"),
            "Cargo.toml must still include cfg(frb_expand); got:\n{}",
            file.content
        );
        assert!(
            !file.content.contains("values("),
            "Cargo.toml must not contain check-cfg values() — forwarding replaces allow-list; got:\n{}",
            file.content
        );
        toml::from_str::<toml::Value>(&file.content).expect("generated Cargo.toml must be valid TOML");
    }

    /// End-to-end regression for the two derivation sites that previously disagreed: the dart
    /// crate's `lib.rs` (via `bridge_fn::emit_bridge_fn`, which renders `f.cfg` verbatim as
    /// `#[cfg(...)]`) and its `Cargo.toml` `[features]` table (via `cargo::emit_cargo_toml`, which
    /// declares whatever `shared_cfg::collect_cfg_features` returns). Runs the real
    /// `gen_rust_crate::emit` entry point — not hand-built strings — on a surface with a plain
    /// cfg-gated function (`native-http`, already covered before this fix) alongside a service
    /// whose configurator is cfg-gated (`tower`, previously dropped because `collect_cfg_features`
    /// never walked `ApiSurface::services`). ~keep
    #[test]
    fn emit_agrees_on_cfg_features_between_lib_rs_and_cargo_toml_including_services() {
        use crate::core::config::ResolvedCrateConfig;
        use crate::core::ir::{ApiSurface, FunctionDef, MethodDef, ServiceDef};

        let api = ApiSurface {
            functions: vec![FunctionDef {
                name: "native_ping".to_string(),
                rust_path: "sample_lib::native_ping".to_string(),
                cfg: Some(r#"feature = "native-http""#.to_string()),
                ..Default::default()
            }],
            services: vec![ServiceDef {
                name: "ClientConfig".to_string(),
                rust_path: "sample_lib::client::ClientConfig".to_string(),
                constructor: MethodDef {
                    name: "new".to_string(),
                    ..Default::default()
                },
                configurators: vec![MethodDef {
                    name: "with_tower_layer".to_string(),
                    cfg: Some(r#"feature = "tower""#.to_string()),
                    ..Default::default()
                }],
                registrations: vec![],
                entrypoints: vec![],
                doc: String::new(),
                cfg: None,
            }],
            ..Default::default()
        };
        let config = ResolvedCrateConfig {
            name: "sample-lib".to_string(),
            ..Default::default()
        };

        let files = crate::backends::dart::gen_rust_crate::emit(&api, &config).expect("dart backend generates files");
        let lib_rs = files
            .iter()
            .find(|f| f.path.to_string_lossy().ends_with("src/lib.rs"))
            .expect("lib.rs is generated");
        let cargo_toml = files
            .iter()
            .find(|f| f.path.to_string_lossy().ends_with("Cargo.toml"))
            .expect("Cargo.toml is generated");

        // Control (deliverable: a negative assertion is worthless unless the fixture actually
        // emitted a cfg gate) — prove `native_ping` really is gated in the generated source before
        // trusting any conclusion drawn from the manifest.
        assert!(
            lib_rs.content.contains("#[cfg(feature = \"native-http\")]") && lib_rs.content.contains("fn native_ping"),
            "control: the fixture must actually emit a cfg-gated function in lib.rs, got:\n{}",
            lib_rs.content
        );

        let parsed: toml::Value = toml::from_str(&cargo_toml.content).expect("generated Cargo.toml must be valid TOML");
        let declared: Vec<&str> = parsed["features"]
            .as_table()
            .expect("[features] is a table")
            .keys()
            .map(String::as_str)
            .collect();

        assert!(
            declared.contains(&"native-http"),
            "control: the function-level gate must already forward, got: {declared:?}"
        );
        assert!(
            declared.contains(&"tower"),
            "the service configurator's cfg gate must also forward to [features], or cargo rejects \
             any `#[cfg(feature = \"tower\")]` a backend re-emits for it as unexpected_cfg; got: {declared:?}"
        );
    }

    /// The dart scaffold already emits its own `unexpected_cfgs` check-cfg
    /// allowlist for `cfg(frb_expand)` into `[lints.rust]`. A configured
    /// `[crates.cargo_lints]` table must compose with that single table -- not
    /// open a second `[lints.rust]` header, which Cargo rejects as a duplicate
    /// table -- and the builtin `cfg(frb_expand)` entry must survive a colliding
    /// user key.
    #[test]
    fn cargo_toml_merges_configured_cargo_lints_with_builtin_unexpected_cfgs() {
        use crate::core::config::{CargoLintsConfig, ResolvedCrateConfig};
        use crate::core::ir::ApiSurface;

        let mut cargo_lints = CargoLintsConfig::default();
        cargo_lints
            .rust
            .insert("unexpected_cfgs".to_string(), toml::Value::String("warn".to_string()));
        cargo_lints
            .rust
            .insert("unused_must_use".to_string(), toml::Value::String("deny".to_string()));
        cargo_lints
            .clippy
            .insert("print_stdout".to_string(), toml::Value::String("deny".to_string()));
        let config = ResolvedCrateConfig {
            name: "sample-lib".to_string(),
            cargo_lints,
            ..Default::default()
        };
        let file = emit_cargo_toml("packages/dart/rust", &ApiSurface::default(), &config, "sample_lib");

        assert_eq!(
            file.content.matches("[lints.rust]").count(),
            1,
            "must not emit a second [lints.rust] table; got:\n{}",
            file.content
        );
        assert!(
            file.content
                .contains("unexpected_cfgs = { level = \"warn\", check-cfg = ['cfg(frb_expand)'] }"),
            "the builtin cfg(frb_expand) entry must survive the user's colliding key; got:\n{}",
            file.content
        );
        assert!(
            file.content.contains("unused_must_use = \"deny\""),
            "non-colliding configured rust lints must be spliced in; got:\n{}",
            file.content
        );
        assert!(
            file.content
                .contains("[lints.clippy]\ndbg_macro = \"deny\"\nprint_stderr = \"deny\"\nprint_stdout = \"deny\""),
            "configured clippy lints must merge with the builtin deny defaults; got:\n{}",
            file.content
        );
        toml::from_str::<toml::Value>(&file.content).expect("generated Cargo.toml must be valid TOML");
    }

    /// Absence of a configured `cargo_lints` table must reproduce the pre-existing
    /// builtin `[lints.rust]` block exactly, followed by the builtin `[lints.clippy]`
    /// deny block that alef now emits unconditionally for every generated binding crate.
    #[test]
    fn cargo_toml_emits_builtin_clippy_denies_when_cargo_lints_unset() {
        use crate::core::config::ResolvedCrateConfig;
        use crate::core::ir::ApiSurface;

        let config = ResolvedCrateConfig {
            name: "sample-lib".to_string(),
            ..Default::default()
        };
        let file = emit_cargo_toml("packages/dart/rust", &ApiSurface::default(), &config, "sample_lib");

        assert!(
            file.content.ends_with(
                "[lints.rust]\n# flutter_rust_bridge uses #[cfg(frb_expand)] internally during macro expansion.\n\
                 unexpected_cfgs = { level = \"warn\", check-cfg = ['cfg(frb_expand)'] }\n\n\
                 [lints.clippy]\ndbg_macro = \"deny\"\nprint_stderr = \"deny\"\nprint_stdout = \"deny\"\n"
            ),
            "got:\n{}",
            file.content
        );
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

    /// Features listed under `excluded_default_features` must still be declared
    /// as opt-in forwarding entries, but must NOT appear in the `default = [...]`
    /// array. This keeps `cargo build --features <name>` working on desktop
    /// while preventing default builds (e.g. iOS / Android NDK cross-compiles)
    /// from auto-activating features that pull in system libraries with
    /// cross-compile-hostile `build.rs` scripts (e.g. `libheif-sys` via `heic`).
    #[test]
    fn cargo_toml_excludes_named_features_from_default_but_keeps_forwarding_entries() {
        use crate::core::config::ResolvedCrateConfig;
        use crate::core::config::languages::DartConfig;
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
            dart: Some(DartConfig {
                excluded_default_features: vec!["heic".to_string()],
                ..Default::default()
            }),
            ..Default::default()
        };
        let file = emit_cargo_toml("packages/dart/rust", &api, &config, "sample_lib");

        assert!(
            file.content.contains(r#"heic = ["sample_lib/heic"]"#),
            "Cargo.toml must keep `heic` forwarding entry; got:\n{}",
            file.content
        );
        assert!(
            file.content.contains(r#"svg = ["sample_lib/svg"]"#),
            "Cargo.toml must keep `svg` forwarding entry; got:\n{}",
            file.content
        );
        let default_line = file
            .content
            .lines()
            .find(|l| l.starts_with("default = ["))
            .expect("default = [...] line must be emitted");
        assert!(
            !default_line.contains("\"heic\""),
            "default = [...] must NOT contain excluded `heic`; got: {default_line}"
        );
        assert!(
            default_line.contains("\"svg\""),
            "default = [...] must still contain non-excluded `svg`; got: {default_line}"
        );
        toml::from_str::<toml::Value>(&file.content).expect("generated Cargo.toml must be valid TOML");
    }

    /// When the core crate defines an `android-target` aggregate, the dart bridge
    /// crate's `[features]` block must emit a matching `android-target` that
    /// forwards to the core dep and enables the cfg-forwarded passthrough features
    /// that are members of the core aggregate (sorted, `full` and non-members
    /// excluded). The forward uses the dart `core_dep_key` (rust-ident form).
    #[test]
    fn cargo_toml_emits_android_target_aggregate_when_core_defines_it() {
        use crate::core::config::ResolvedCrateConfig;
        use crate::core::ir::ApiSurface;
        use std::fs;
        use std::path::PathBuf;
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nresolver = \"2\"\nmembers = [\"crates/sample-core\"]\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("crates/sample-core/src")).unwrap();
        fs::write(root.join("crates/sample-core/src/lib.rs"), "pub fn f() {}").unwrap();
        fs::write(
            root.join("crates/sample-core/Cargo.toml"),
            r#"[package]
name = "sample-core"
version = "0.1.0"

[features]
android-target = ["no-ort-target", "ocr"]
no-ort-target = ["pdf", "html"]
pdf = []
html = []
ocr = []
embeddings = []
"#,
        )
        .unwrap();

        let api = ApiSurface {
            enums: vec![EnumDef {
                name: "Format".to_string(),
                variants: vec![
                    make_unit_variant("Pdf", Some("feature = \"pdf\"")),
                    make_unit_variant("Html", Some("feature = \"html\"")),
                    make_unit_variant("Ocr", Some("feature = \"ocr\"")),
                    make_unit_variant("Embeddings", Some("feature = \"embeddings\"")),
                ],
                ..Default::default()
            }],
            ..Default::default()
        };
        let config = ResolvedCrateConfig {
            name: "sample-core".to_string(),
            workspace_root: Some(root.to_path_buf()),
            sources: vec![PathBuf::from("crates/sample-core/src/lib.rs")],
            ..Default::default()
        };
        let file = emit_cargo_toml("packages/dart/rust", &api, &config, "sample_core");
        assert!(
            file.content
                .contains(r#"android-target = ["sample_core/android-target", "html", "ocr", "pdf"]"#),
            "dart Cargo.toml must emit the android-target aggregate feature; got:\n{}",
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

    /// Regression test: `cargo-sort` (and hence `poly lint`) orders
    /// `[target.'cfg(...)'.dependencies]` tables alphabetically by the raw cfg
    /// predicate string (plain byte-wise comparison), NOT with the default
    /// `cfg(not(any(...)))` branch always first. With multiple overrides, an
    /// `all(...)`-prefixed override (a downstream consumer's macOS-Intel target)
    /// must sort *before* the `not(any(...))` default branch (`'a'` < `'n'`), while a
    /// `target_os = ...` override sorts after it (`'n'` < `'t'`).
    #[test]
    fn cargo_toml_target_dep_overrides_sort_all_before_not() {
        use crate::core::config::ResolvedCrateConfig;
        use crate::core::config::languages::{DartConfig, DartTargetDepOverride};
        use crate::core::ir::ApiSurface;

        let api = ApiSurface::default();
        let config = ResolvedCrateConfig {
            name: "sample-lib".to_string(),
            dart: Some(DartConfig {
                target_dep_overrides: vec![
                    DartTargetDepOverride {
                        cfg: "target_os = \"android\"".to_string(),
                        features: vec!["android-target".to_string()],
                        default_features: false,
                    },
                    DartTargetDepOverride {
                        cfg: "target_os = \"windows\"".to_string(),
                        features: vec!["windows-target".to_string()],
                        default_features: false,
                    },
                    DartTargetDepOverride {
                        cfg: "all(target_os = \"macos\", target_arch = \"x86_64\")".to_string(),
                        features: vec!["macos-intel-target".to_string()],
                        default_features: false,
                    },
                ],
                ..Default::default()
            }),
            ..Default::default()
        };
        let file = emit_cargo_toml("packages/dart/rust", &api, &config, "sample_lib");
        let content = &file.content;

        let all_pos = content
            .find("[target.'cfg(all(target_os = \"macos\", target_arch = \"x86_64\"))'.dependencies]")
            .expect("expected the macOS-Intel `all(...)` override block");
        let not_pos = content
            .find("[target.'cfg(not(any(")
            .expect("expected the default `not(any(...))` block");
        let android_pos = content
            .find("[target.'cfg(target_os = \"android\")'.dependencies]")
            .expect("expected the android override block");

        assert!(
            all_pos < not_pos,
            "the `all(...)` override must sort BEFORE the `not(...)` default branch; got:\n{content}"
        );
        assert!(
            not_pos < android_pos,
            "the `not(...)` default branch must sort before `target_os = \"android\"`; got:\n{content}"
        );
        toml::from_str::<toml::Value>(content).expect("generated Cargo.toml must be valid TOML");
    }

    /// A consumer whose root `Cargo.toml` declares a `[workspace]` but no
    /// `[workspace.package] version` (e.g. a plain `resolver`+`members` table) must
    /// still get a literal `version = "…"` line — there is nothing to inherit from.
    #[test]
    fn cargo_toml_uses_literal_version_without_workspace_package_version() {
        use crate::core::config::ResolvedCrateConfig;
        use crate::core::ir::ApiSurface;
        use std::fs;
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let root_cargo_toml = root.join("Cargo.toml");
        fs::write(
            &root_cargo_toml,
            "[workspace]\nresolver = \"2\"\nmembers = [\"crates/sample-core\"]\n",
        )
        .unwrap();

        let api = ApiSurface::default();
        let config = ResolvedCrateConfig {
            name: "sample-core".to_string(),
            workspace_root: Some(root.to_path_buf()),
            version_from: root_cargo_toml.to_string_lossy().into_owned(),
            ..Default::default()
        };
        let file = emit_cargo_toml("packages/dart/rust", &api, &config, "sample_core");
        assert!(
            file.content.contains("version = \"0.1.0\""),
            "must fall back to a literal version when [workspace.package] is absent; got:\n{}",
            file.content
        );
        assert!(
            !file.content.contains("version.workspace = true"),
            "must not claim workspace inheritance that doesn't exist; got:\n{}",
            file.content
        );
        toml::from_str::<toml::Value>(&file.content).expect("generated Cargo.toml must be valid TOML");
    }

    /// A consumer whose root `Cargo.toml` declares `[workspace.package] version` must
    /// get `version.workspace = true` in the emitted dart rust crate's Cargo.toml, so
    /// the crate tracks the workspace-wide version bump instead of drifting behind a
    /// stamped-once literal.
    #[test]
    fn cargo_toml_inherits_workspace_version_when_workspace_package_declares_it() {
        use crate::core::config::ResolvedCrateConfig;
        use crate::core::ir::ApiSurface;
        use std::fs;
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let root_cargo_toml = root.join("Cargo.toml");
        fs::write(
            &root_cargo_toml,
            "[workspace]\nresolver = \"2\"\nmembers = [\"crates/sample-core\"]\n\n[workspace.package]\nversion = \"9.9.9\"\n",
        )
        .unwrap();

        let api = ApiSurface::default();
        let config = ResolvedCrateConfig {
            name: "sample-core".to_string(),
            workspace_root: Some(root.to_path_buf()),
            version_from: root_cargo_toml.to_string_lossy().into_owned(),
            ..Default::default()
        };
        let file = emit_cargo_toml("packages/dart/rust", &api, &config, "sample_core");
        assert!(
            file.content.contains("version.workspace = true"),
            "must inherit the workspace version when [workspace.package] declares one; got:\n{}",
            file.content
        );
        assert!(
            !file.content.contains("version = \"9.9.9\""),
            "must not also emit a literal version line; got:\n{}",
            file.content
        );
        toml::from_str::<toml::Value>(&file.content).expect("generated Cargo.toml must be valid TOML");
    }
}
