use crate::codegen::cfg::enabled_features_for_language;
use crate::codegen::shared;
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, TypeDef};
use ahash::{AHashMap, AHashSet};
use std::collections::BTreeSet;

pub(crate) struct EffectiveWasmSurface {
    pub(crate) api: ApiSurface,
    pub(crate) exclude_types: Vec<String>,
    pub(crate) unknown_type_omissions: AHashMap<String, Vec<(String, String)>>,
    source_types: Vec<TypeDef>,
}

impl EffectiveWasmSurface {
    /// Retain omitted field metadata as `binding_excluded` so E2E rendering can distinguish a
    /// real-but-unexported setter from a legitimate free-form fixture key. ~keep
    pub(crate) fn emitted_types(&self) -> Vec<TypeDef> {
        let mut types = self.source_types.clone();
        types.retain(|definition| !self.exclude_types.contains(&definition.name));
        for definition in &mut types {
            let effective = self
                .api
                .types
                .iter()
                .find(|candidate| candidate.name == definition.name && candidate.rust_path == definition.rust_path);
            for field in &mut definition.fields {
                let removed_from_surface = effective.is_none_or(|candidate| {
                    !candidate
                        .fields
                        .iter()
                        .any(|effective_field| effective_field.name == field.name)
                });
                field.binding_excluded = field.binding_excluded
                    || removed_from_surface
                    || super::cfg::field_references_excluded_type(&field.ty, &self.exclude_types);
            }
        }
        types
    }
}

pub(crate) fn effective_wasm_surface(api: &ApiSurface, config: &ResolvedCrateConfig) -> EffectiveWasmSurface {
    let enabled_features = enabled_features_for_language(config, Language::Wasm);
    let mut excluded = known_exclusions(api, config, &enabled_features);
    let filtered = filter_wasm_fields(api, config, &enabled_features);
    let known_type_names = known_type_names(&filtered, config);
    let unknown_type_omissions = collect_unknown_omissions(&filtered, &known_type_names, &mut excluded);

    EffectiveWasmSurface {
        api: filtered,
        exclude_types: excluded.into_iter().collect(),
        unknown_type_omissions,
        source_types: api.types.clone(),
    }
}

fn known_exclusions(api: &ApiSurface, config: &ResolvedCrateConfig, enabled_features: &[String]) -> BTreeSet<String> {
    let mut excluded: BTreeSet<String> = super::ts_union::wasm_exclude_types(config).into_iter().collect();

    excluded.extend(
        api.types
            .iter()
            .filter(|definition| super::cfg::is_gated_behind_disabled_feature(&definition.cfg, enabled_features))
            .map(|definition| definition.name.clone()),
    );
    excluded.extend(
        api.enums
            .iter()
            .filter(|definition| super::cfg::is_gated_behind_disabled_feature(&definition.cfg, enabled_features))
            .map(|definition| definition.name.clone()),
    );
    extend_dropped_crate_exclusions(&mut excluded, api, &dropped_crates(config));

    excluded
}

fn dropped_crates(config: &ResolvedCrateConfig) -> BTreeSet<String> {
    let core_import = config.core_import_for_language(Language::Wasm);
    let remapped_crates: BTreeSet<String> = config
        .wasm
        .as_ref()
        .map(|wasm| wasm.source_crate_remaps.iter())
        .into_iter()
        .flatten()
        .map(|name| name.replace('-', "_"))
        .collect();
    config
        .wasm
        .as_ref()
        .map(|wasm| wasm.exclude_extra_dependencies.iter())
        .into_iter()
        .flatten()
        .map(|name| name.replace('-', "_"))
        .filter(|name| name != &core_import && !remapped_crates.contains(name))
        .collect()
}

fn extend_dropped_crate_exclusions(
    excluded: &mut BTreeSet<String>,
    api: &ApiSurface,
    dropped_crates: &BTreeSet<String>,
) {
    excluded.extend(
        api.types
            .iter()
            .filter(|definition| dropped_crates.contains(&source_crate(&definition.rust_path)))
            .map(|definition| definition.name.clone()),
    );
    excluded.extend(
        api.enums
            .iter()
            .filter(|definition| dropped_crates.contains(&source_crate(&definition.rust_path)))
            .map(|definition| definition.name.clone()),
    );
    excluded.extend(
        api.errors
            .iter()
            .filter(|definition| dropped_crates.contains(&source_crate(&definition.rust_path)))
            .map(|definition| definition.name.clone()),
    );
}

fn filter_wasm_fields(api: &ApiSurface, config: &ResolvedCrateConfig, enabled_features: &[String]) -> ApiSurface {
    let mut filtered = api.clone();
    if let Some(wasm) = &config.wasm {
        for definition in &mut filtered.types {
            if let Some(skip_list) = wasm.exclude_fields.get(&definition.name) {
                let before = definition.fields.len();
                definition.fields.retain(|field| !skip_list.contains(&field.name));
                if definition.fields.len() != before {
                    definition.has_stripped_cfg_fields = true;
                }
            }
        }
    }
    super::types::filter_cfg_fields_for_features(&filtered, enabled_features)
}

fn known_type_names(api: &ApiSurface, config: &ResolvedCrateConfig) -> AHashSet<String> {
    let mut known_type_names: AHashSet<String> = api.types.iter().map(|definition| definition.name.clone()).collect();
    known_type_names.extend(api.enums.iter().map(|definition| definition.name.clone()));
    if let Some(wasm) = &config.wasm {
        known_type_names.extend(wasm.type_overrides.keys().cloned());
    }
    known_type_names.extend(config.untagged_union_text_types.iter().cloned());
    known_type_names
}

fn collect_unknown_omissions(
    api: &ApiSurface,
    known_type_names: &AHashSet<String>,
    excluded: &mut BTreeSet<String>,
) -> AHashMap<String, Vec<(String, String)>> {
    let mut unknown_type_omissions: AHashMap<String, Vec<(String, String)>> = AHashMap::default();
    for definition in &api.types {
        if definition.is_opaque || definition.is_trait || excluded.contains(&definition.name) {
            continue;
        }
        for field in shared::binding_fields(&definition.fields) {
            let excluded_vec: Vec<String> = excluded.iter().cloned().collect();
            if super::cfg::field_references_excluded_type(&field.ty, &excluded_vec) {
                continue;
            }
            let Some(unknown_name) = super::cfg::first_unknown_named_type(&field.ty, known_type_names) else {
                continue;
            };
            excluded.insert(unknown_name.to_string());
            unknown_type_omissions
                .entry(definition.name.clone())
                .or_default()
                .push((field.name.clone(), unknown_name.to_string()));
        }
    }
    unknown_type_omissions
}

fn source_crate(rust_path: &str) -> String {
    rust_path.split("::").next().unwrap_or("").replace('-', "_")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{EnumDef, ErrorDef};

    #[test]
    fn effective_surface_removes_fields_behind_disabled_wasm_features() {
        let mut field = crate::core::ir::FieldDef {
            name: "tree_sitter".into(),
            ty: crate::core::ir::TypeRef::Named("TreeSitterConfig".into()),
            ..Default::default()
        };
        field.cfg = Some(r#"feature = "tree-sitter""#.into());
        let api = ApiSurface {
            types: vec![
                TypeDef {
                    name: "ExtractionConfig".into(),
                    fields: vec![field],
                    ..Default::default()
                },
                TypeDef {
                    name: "TreeSitterConfig".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };

        let surface = effective_wasm_surface(&api, &ResolvedCrateConfig::default());

        assert!(surface.exclude_types.is_empty());
        assert!(surface.api.types[0].fields.is_empty());
    }

    #[test]
    fn effective_exclusions_include_every_ir_kind_from_a_dropped_crate() {
        let rust_path = "tree_sitter_language_pack::api".to_string();
        let types = [TypeDef {
            name: "TreeSitterConfig".into(),
            rust_path: rust_path.clone(),
            ..Default::default()
        }];
        let enums = [EnumDef {
            name: "TreeSitterMode".into(),
            rust_path: rust_path.clone(),
            ..Default::default()
        }];
        let errors = [ErrorDef {
            name: "TreeSitterError".into(),
            rust_path,
            original_rust_path: String::new(),
            variants: Vec::new(),
            doc: String::new(),
            methods: Vec::new(),
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }];
        let wasm =
            toml::from_str("exclude_extra_dependencies = [\"tree-sitter-language-pack\"]").expect("WASM config parses");
        let config = ResolvedCrateConfig {
            wasm: Some(wasm),
            ..Default::default()
        };

        let api = ApiSurface {
            types: types.into(),
            enums: enums.into(),
            errors: errors.into(),
            ..Default::default()
        };
        let surface = effective_wasm_surface(&api, &config);

        assert_eq!(
            surface.exclude_types,
            ["TreeSitterConfig", "TreeSitterError", "TreeSitterMode"]
        );
        assert!(surface.emitted_types().is_empty());
    }

    #[test]
    fn effective_surface_records_unknown_field_references() {
        let api = ApiSurface {
            types: vec![TypeDef {
                name: "ExtractionConfig".into(),
                fields: vec![crate::core::ir::FieldDef {
                    name: "foreign".into(),
                    ty: crate::core::ir::TypeRef::Named("ForeignConfig".into()),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };

        let surface = effective_wasm_surface(&api, &ResolvedCrateConfig::default());

        assert_eq!(surface.exclude_types, ["ForeignConfig"]);
        assert_eq!(
            surface.unknown_type_omissions.get("ExtractionConfig"),
            Some(&vec![("foreign".to_string(), "ForeignConfig".to_string())])
        );
        assert!(surface.emitted_types()[0].fields[0].binding_excluded);
    }

    #[test]
    fn e2e_field_metadata_marks_every_omitted_flatten_source() {
        let mut binding_excluded = crate::core::ir::FieldDef {
            name: "binding_excluded".into(),
            ty: crate::core::ir::TypeRef::Json,
            serde_flatten: true,
            ..Default::default()
        };
        binding_excluded.binding_excluded = true;
        let mut configured = crate::core::ir::FieldDef {
            name: "configured".into(),
            ty: crate::core::ir::TypeRef::Json,
            serde_flatten: true,
            ..Default::default()
        };
        configured.binding_excluded = false;
        let mut cfg_disabled = crate::core::ir::FieldDef {
            name: "cfg_disabled".into(),
            ty: crate::core::ir::TypeRef::Json,
            serde_flatten: true,
            ..Default::default()
        };
        cfg_disabled.cfg = Some(r#"feature = "disabled""#.into());
        let dropped_type = crate::core::ir::FieldDef {
            name: "dropped_type".into(),
            ty: crate::core::ir::TypeRef::Named("ExternalOptions".into()),
            serde_flatten: true,
            ..Default::default()
        };
        let api = ApiSurface {
            types: vec![
                TypeDef {
                    name: "Options".into(),
                    fields: vec![binding_excluded, configured, cfg_disabled, dropped_type],
                    ..Default::default()
                },
                TypeDef {
                    name: "ExternalOptions".into(),
                    rust_path: "external::ExternalOptions".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let wasm = toml::from_str(
            "exclude_extra_dependencies = [\"external\"]\nexclude_fields = { Options = [\"configured\"] }",
        )
        .expect("WASM config parses");
        let config = ResolvedCrateConfig {
            wasm: Some(wasm),
            ..Default::default()
        };

        let fields = effective_wasm_surface(&api, &config).emitted_types()[0].fields.clone();

        assert_eq!(fields.len(), 4);
        assert!(fields.iter().all(|field| field.binding_excluded));
    }
}
