mod binding_file;
mod constructors;
mod functions;
mod methods;
mod service_api;
pub(super) mod types;

use binding_file::{find_options_bridge_function, format_go_code, gen_go_file, strip_trailing_whitespace};

use crate::core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile};
use crate::core::config::{AdapterPattern, BridgeBinding, Language, ResolvedCrateConfig, resolve_output_dir};
use crate::core::ir::ApiSurface;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

pub struct GoBackend;

impl GoBackend {
    /// Extract the package name from module path (last segment).
    /// Sanitize by removing hyphens and converting to lowercase.
    fn package_name(module_path: &str) -> String {
        module_path
            .split('/')
            .next_back()
            .unwrap_or("binding")
            .replace('-', "")
            .to_lowercase()
    }
}

impl Backend for GoBackend {
    fn name(&self) -> &str {
        "go"
    }

    fn language(&self) -> Language {
        Language::Go
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            supports_async: true,
            supports_classes: true,
            supports_enums: true,
            supports_option: true,
            supports_result: true,
            supports_service_api: true,
            ..Capabilities::default()
        }
    }

    fn generate_bindings(&self, api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        let deduped_api = api.with_deduped_functions();
        let api = &deduped_api;
        let module_path = config.go_module();
        let pkg_name = config
            .go
            .as_ref()
            .and_then(|g| g.package_name.clone())
            .unwrap_or_else(|| Self::package_name(&module_path));
        let ffi_prefix = config.ffi_prefix();

        let output_dir = {
            let mut d = resolve_output_dir(config.output_paths.get("go"), &config.name, "packages/go/");
            if !d.ends_with('/') {
                d.push('/');
            }
            d
        };

        let ffi_lib_name = config.ffi_lib_name();
        let ffi_header = config.ffi_header_name();
        let ffi_crate_dir = config
            .output_paths
            .get("ffi")
            .and_then(|p| {
                let path = p.as_path();
                path.ancestors()
                    .find(|a| {
                        a.file_name()
                            .is_some_and(|n| n != "src" && n != "lib" && n != "include")
                    })
                    .map(|a| a.to_string_lossy().to_string())
            })
            .unwrap_or_else(|| format!("crates/{ffi_lib_name}"));
        let bridge_param_names: HashSet<String> = config
            .trait_bridges
            .iter()
            .filter_map(|b| b.param_name.clone())
            .collect();
        let bridge_type_aliases: HashSet<String> = config
            .trait_bridges
            .iter()
            .filter_map(|b| b.type_alias.clone())
            .collect();
        let visitor_callbacks_enabled = config.ffi.as_ref().is_some_and(|f| f.visitor_callbacks);
        let visitor_bridge_cfg = config
            .trait_bridges
            .iter()
            .find(|b| b.bind_via == BridgeBinding::OptionsField);
        let has_options_field_bridge = visitor_bridge_cfg.is_some();
        let has_visitor_bridge =
            has_options_field_bridge || (!config.trait_bridges.is_empty() && visitor_callbacks_enabled);

        let has_plugin_bridges = config.trait_bridges.iter().any(|b| b.register_fn.is_some());

        let streaming_methods: HashMap<(String, String), String> = config
            .adapters
            .iter()
            .filter(|a| matches!(a.pattern, AdapterPattern::Streaming))
            .filter_map(|a| {
                let owner = a.owner_type.clone()?;
                let item = a.item_type.clone()?;
                Some(((owner, a.name.clone()), item))
            })
            .collect();

        let ffi_exclude_functions: HashSet<String> = config
            .ffi
            .as_ref()
            .map(|f| f.exclude_functions.iter().cloned().collect())
            .unwrap_or_default();
        let mut exclude_types: HashSet<String> = config
            .ffi
            .as_ref()
            .map(|f| f.exclude_types.iter().cloned().collect())
            .unwrap_or_default();
        if let Some(go_config) = &config.go {
            exclude_types.extend(go_config.exclude_types.iter().cloned());
        }
        exclude_types.extend(api.types.iter().filter(|t| t.binding_excluded).map(|t| t.name.clone()));
        exclude_types.extend(
            config
                .opaque_types
                .iter()
                .filter(|(_, path)| path.contains('<'))
                .map(|(name, _)| name.clone()),
        );

        let value_only_types: HashSet<String> = api
            .types
            .iter()
            .filter(|t| !t.is_opaque && t.fields.iter().all(|f| {
                matches!(f.ty, crate::core::ir::TypeRef::Primitive(_) | crate::core::ir::TypeRef::String | crate::core::ir::TypeRef::Char | crate::core::ir::TypeRef::Path)
                    || matches!(&f.ty, crate::core::ir::TypeRef::Optional(inner) if matches!(inner.as_ref(), crate::core::ir::TypeRef::Primitive(_) | crate::core::ir::TypeRef::String | crate::core::ir::TypeRef::Char | crate::core::ir::TypeRef::Path))
            }))
            .map(|t| t.name.clone())
            .collect();

        let content = format_go_code(&strip_trailing_whitespace(&gen_go_file(
            api,
            config,
            &ffi_prefix,
            &pkg_name,
            &ffi_lib_name,
            &ffi_header,
            &ffi_crate_dir,
            &output_dir,
            &bridge_param_names,
            &bridge_type_aliases,
            &streaming_methods,
            &ffi_exclude_functions,
            &exclude_types,
            &value_only_types,
            visitor_bridge_cfg,
        )));

        let _adapter_bodies = crate::adapters::build_adapter_bodies(config, Language::Go)?;

        let depth = output_dir.trim_end_matches('/').matches('/').count() + 1;
        let to_root = "../".repeat(depth);

        let mut files = vec![GeneratedFile {
            path: PathBuf::from(format!("{output_dir}binding.go")),
            content,
            generated_header: true,
        }];

        if has_visitor_bridge && let Some(bridge_cfg) = visitor_bridge_cfg {
            let Some(options_field) = bridge_cfg.resolved_options_field() else {
                return Err(crate::core::AlefError::Config(
                    "Go visitor generation requires trait bridge options_field metadata".to_string(),
                )
                .into());
            };
            let vtable_trait_name = bridge_cfg.trait_name.clone();
            let options_field = options_field.to_string();

            let trait_map: HashMap<&str, &crate::core::ir::TypeDef> = api
                .types
                .iter()
                .filter(|t| t.is_trait)
                .map(|t| (t.name.as_str(), t))
                .collect();
            let visitor_trait = trait_map.get(bridge_cfg.trait_name.as_str()).copied();
            let visitor_function = find_options_bridge_function(api, bridge_cfg);

            let visitor_content = if let (Some(vt), Some(visitor_func)) = (visitor_trait, visitor_function) {
                strip_trailing_whitespace(&crate::backends::go::gen_visitor::gen_visitor_file(
                    api,
                    &pkg_name,
                    &ffi_prefix,
                    &ffi_header,
                    &ffi_crate_dir,
                    &to_root,
                    &vtable_trait_name,
                    &options_field,
                    vt,
                    bridge_cfg,
                    visitor_func,
                ))
            } else {
                eprintln!(
                    "[alef] gen_visitor_file(go): visitor bridge `{vtable_trait_name}` missing trait or options function in IR, skipping visitor.go"
                );
                String::new()
            };
            files.push(GeneratedFile {
                path: PathBuf::from(format!("{output_dir}visitor.go")),
                content: visitor_content,
                generated_header: true,
            });
        }

        if has_plugin_bridges {
            let trait_bridges_content = strip_trailing_whitespace(&super::trait_bridge::gen_trait_bridges_file(
                api,
                config,
                &pkg_name,
                &ffi_prefix,
                &ffi_header,
                &ffi_crate_dir,
                &to_root,
                &config.name,
            ));
            if !trait_bridges_content.trim().is_empty() && trait_bridges_content.len() > 100 {
                files.push(GeneratedFile {
                    path: PathBuf::from(format!("{output_dir}trait_bridges.go")),
                    content: trait_bridges_content,
                    generated_header: true,
                });
            }
        }

        // Generate generate.go with //go:generate directive for FFI library download
        let generate_go_content =
            crate::backends::go::template_env::render("generate_cgo_flags.go.jinja", minijinja::context! {});
        files.push(GeneratedFile {
            path: PathBuf::from(format!("{output_dir}generate.go")),
            content: generate_go_content,
            generated_header: false,
        });

        let crate_version = api.version.to_string();
        let repo_url = config.github_repo();
        let asset_prefix = config.name.clone();
        let download_tool_content = crate::backends::go::template_env::render(
            "cmd_download_ffi_main.go.jinja",
            minijinja::context! {
                ffi_lib_name => &ffi_lib_name,
                crate_version => &crate_version,
                repo_url => &repo_url,
                asset_prefix => &asset_prefix,
            },
        );
        files.push(GeneratedFile {
            path: PathBuf::from(format!("{output_dir}cmd/download_ffi/main.go")),
            content: download_tool_content,
            generated_header: false,
        });

        // Generate embed_ffi.go with //go:embed directive to ensure header files
        let embed_ffi_content = crate::backends::go::template_env::render(
            "embed_ffi.go.jinja",
            minijinja::context! {
                pkg_name => &pkg_name,
            },
        );
        files.push(GeneratedFile {
            path: PathBuf::from(format!("{output_dir}embed_ffi.go")),
            content: embed_ffi_content,
            generated_header: false,
        });

        Ok(files)
    }

    /// Go bindings are already the public API (single .go file wrapping C FFI).
    /// This returns empty since the binding.go file serves as both the FFI layer
    /// and the high-level public API for consumers.
    fn generate_public_api(
        &self,
        _api: &ApiSurface,
        _config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        Ok(vec![])
    }

    fn generate_service_api(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        let module_path = config.go_module();
        let pkg_name = config
            .go
            .as_ref()
            .and_then(|g| g.package_name.clone())
            .unwrap_or_else(|| Self::package_name(&module_path));
        let ffi_prefix = config.ffi_prefix();

        service_api::generate(api, config, &pkg_name, &ffi_prefix)
    }

    fn build_config(&self) -> Option<BuildConfig> {
        Some(BuildConfig {
            tool: "go",
            crate_suffix: "",
            build_dep: BuildDependency::Ffi,
            post_build: vec![],
        })
    }
}

#[cfg(test)]
mod tests;
