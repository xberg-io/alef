mod binding_file;
mod components;
mod constructors;
mod functions;
mod methods;
mod result_presence;
mod service_api;
pub(super) mod types;

use binding_file::{find_options_bridge_function, format_go_code, gen_go_file, strip_trailing_whitespace};
pub(crate) use functions::adapter_flattened_field;

use crate::core::backend::{
    Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile, PostBuildStep, TraitBridgeRegistrationSurface,
};
use crate::core::config::{AdapterPattern, BridgeBinding, Language, ResolvedCrateConfig, resolve_output_dir};
use crate::core::ir::ApiSurface;
use heck::ToShoutySnakeCase;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

pub struct GoBackend;

/// Sanitize a crate name into a valid Go identifier fragment: non-alphanumeric bytes
/// become `_`, and a leading digit is prefixed with `_` (Go identifiers can't start with
/// a digit). Used to build the `cmd/setup`-generated shim's import alias for the binding
/// package (`<go_identifier>nativesetup`).
fn go_identifier(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    match sanitized.chars().next() {
        Some(c) if c.is_ascii_digit() => format!("_{sanitized}"),
        _ => sanitized,
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
        // Order the IR once, before anything reads it: every emission loop below concatenates
        // api.types/enums/functions/errors into a single generated file in Vec order. ~keep
        let sorted_api = crate::backends::ir_order::with_sorted_items(api);
        let api = &sorted_api;
        if let Some(go) = config.go.as_ref() {
            crate::core::config::languages::require_shared_native_runtime(
                &go.capsule_types,
                go.shares_native_runtime,
                "go",
            )?;
        }
        let deduped_api = api.with_deduped_functions();
        crate::codegen::cfg::warn_on_ffi_feature_drift(api, config, Language::Go);
        // Derived from the *unfiltered* surface: the FFI crate defaults every cfg-discovered
        // feature ON, so a gate whose items this Go build filters out is still compiled into the
        // cdylib and still guarded in the header. Emitted once, in `binding.go`'s preamble — cgo
        // merges `#cgo` directives across every file of a package, which is what already lets
        // `service_file_preamble.jinja` include the header with no `-I` of its own. ~keep
        let feature_cflags = crate::backends::go::cgo_features::cgo_feature_cflags(api, config);
        let go_features = crate::codegen::cfg::enabled_features_for_language(config, Language::Go);
        let enabled_features: HashSet<&str> = go_features.iter().map(String::as_str).collect();
        // cgo's `import "C"` binds directly to the symbols the C header declares — an item
        // dropped under `#[cfg(feature = "X")]` from the built FFI library is a link-time
        // failure here (no runtime lazy resolution like C#'s DllImport). Omitting the glue for
        // any function/type/enum (and their cfg-gated fields/variants) that the configured Go
        // feature set doesn't satisfy keeps `binding.go` consistent with what actually got
        // compiled and linked. See `with_cfg_filtered_deep` for the precedent (Swift, Kotlin
        // Android, JNI already apply the same filter before their own codegen). ~keep
        let filtered_api = deduped_api.with_cfg_filtered_deep(&enabled_features);
        let api = &filtered_api;
        let pkg_name = config.go_package_name();
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
            .find(|b| b.bind_via == BridgeBinding::OptionsField && b.is_active_for(&Language::Go.to_string()));
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

        let mut ffi_exclude_functions: HashSet<String> = config
            .ffi
            .as_ref()
            .map(|f| f.exclude_functions.iter().cloned().collect())
            .unwrap_or_default();
        if let Some(go_config) = &config.go {
            ffi_exclude_functions.extend(go_config.exclude_functions.iter().cloned());
        }
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

        let go_file = gen_go_file(
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
            &feature_cflags,
        )?;
        let content = format_go_code(&strip_trailing_whitespace(&go_file));

        let _adapter_bodies = crate::adapters::build_adapter_bodies(config, Language::Go)?;

        let depth = output_dir.trim_end_matches('/').matches('/').count() + 1;
        let to_root = "../".repeat(depth);

        let mut files = vec![GeneratedFile {
            path: PathBuf::from(format!("{output_dir}binding.go")),
            content,
            generated_header: true,
        }];

        if !config.components.is_empty() {
            files.push(GeneratedFile {
                path: PathBuf::from(format!("{output_dir}components.go")),
                content: components::generate(&pkg_name, &ffi_prefix, &ffi_header, &ffi_lib_name, &to_root),
                generated_header: true,
            });
        }

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
                tracing::warn!(
                    "gen_visitor_file(go): visitor bridge `{vtable_trait_name}` missing trait or options function in IR, skipping visitor.go"
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
            ));
            if !trait_bridges_content.trim().is_empty() && trait_bridges_content.len() > 100 {
                files.push(GeneratedFile {
                    path: PathBuf::from(format!("{output_dir}trait_bridges.go")),
                    content: trait_bridges_content,
                    generated_header: true,
                });
            }
        }

        // Generate generate.go with the //go:generate directive that vendors natives
        // into .lib/ for writable checkouts (see cmd/setup -lib-dir).
        let generate_go_content =
            crate::backends::go::template_env::render("generate_cgo_flags.go.jinja", minijinja::context! {});
        // `generated_header: true` even though the body opens with `//go:generate` /
        // `//go:build`: the prepended header is line comments, which a build constraint is
        // allowed to be preceded by, and without a recognized marker this `.go` file is
        // markable-but-unmarked -- invisible to `finalize_hashes` and refused by
        // `write_files_report`'s ownership guard the next time the template changes. ~keep
        files.push(GeneratedFile {
            path: PathBuf::from(format!("{output_dir}generate.go")),
            content: generate_go_content,
            generated_header: true,
        });

        let crate_version = api.version.to_string();
        let repo_url = config.github_repo();
        let asset_prefix = config.name.clone();
        let module_path = config.go_module();
        let version_ident = crate::core::version::to_go_version_ident(&crate_version);
        let shim_filename = format!("{}_cgo_link.go", config.name);
        let env_override_var = format!("{}_GO_NATIVE_BASE_URL", config.name.to_shouty_snake_case());
        let go_ident_crate_name = go_identifier(&config.name);

        // Generate cmd/setup/main.go: downloads the platform native library from GitHub
        // releases into a versioned user-cache dir at runtime and writes a machine-local
        // cgo link shim into a consumer package (see cmd_setup_main.go.jinja doc comment).
        let setup_tool_content = crate::backends::go::template_env::render(
            "cmd_setup_main.go.jinja",
            minijinja::context! {
                ffi_lib_name => &ffi_lib_name,
                crate_version => &crate_version,
                repo_url => &repo_url,
                asset_prefix => &asset_prefix,
                crate_name => &config.name,
                module_path => &module_path,
                version_ident => &version_ident,
                shim_filename => &shim_filename,
                env_override_var => &env_override_var,
                go_ident_crate_name => &go_ident_crate_name,
            },
        );
        files.push(GeneratedFile {
            path: PathBuf::from(format!("{output_dir}cmd/setup/main.go")),
            content: setup_tool_content,
            generated_header: false,
        });

        // Generate native_setup.go with the RequireNativeSetup_<version> sentinel that the
        // cmd/setup-written shim references, turning shim/module version skew into a
        // compile-time error.
        let native_setup_content = crate::backends::go::template_env::render(
            "native_setup.go.jinja",
            minijinja::context! {
                pkg_name => &pkg_name,
                crate_version => &crate_version,
                version_ident => &version_ident,
            },
        );
        // The template's own `// Code generated by alef — DO NOT EDIT.` banner is the Go
        // tooling convention, not an alef marker (`content_has_alef_marker` matches
        // "auto-generated by alef" / "Generated by alef", both case-sensitively), so this
        // file needs the prepended header to be stamped at all. It embeds the crate
        // version, so it must be rewritable on every release -- unmarked it would trip the
        // ownership guard in `write_files_report` and freeze at the first version. ~keep
        files.push(GeneratedFile {
            path: PathBuf::from(format!("{output_dir}native_setup.go")),
            content: native_setup_content,
            generated_header: true,
        });

        // Generate embed_ffi.go with //go:embed directive to ensure C headers are included
        // when this module is vendored (`go mod vendor`).
        let embed_ffi_content = crate::backends::go::template_env::render(
            "embed_ffi.go.jinja",
            minijinja::context! {
                pkg_name => &pkg_name,
            },
        );
        files.push(GeneratedFile {
            path: PathBuf::from(format!("{output_dir}embed_ffi.go")),
            content: embed_ffi_content,
            generated_header: true,
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
        // Order the IR once, before anything reads it: every emission loop below concatenates
        // api.types/enums/functions/errors into a single generated file in Vec order. ~keep
        let sorted_api = crate::backends::ir_order::with_sorted_items(api);
        let api = &sorted_api;
        let pkg_name = config.go_package_name();
        let ffi_prefix = config.ffi_prefix();

        let go_features = crate::codegen::cfg::enabled_features_for_language(config, Language::Go);
        let enabled_features: HashSet<&str> = go_features.iter().map(String::as_str).collect();
        let filtered_api = api.with_cfg_filtered_deep(&enabled_features);

        service_api::generate(&filtered_api, config, &pkg_name, &ffi_prefix)
    }

    fn build_config(&self) -> Option<BuildConfig> {
        Some(BuildConfig {
            tool: "go",
            crate_suffix: "",
            build_dep: BuildDependency::Ffi,
            post_build: vec![PostBuildStep::StageFfiLibrary],
        })
    }

    fn trait_bridge_registration_surface(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> Vec<TraitBridgeRegistrationSurface> {
        super::trait_bridge::registration_surface(api, config)
    }
}

#[cfg(test)]
mod ffi_parity_tests;

#[cfg(test)]
mod service_symbol_parity_tests;

#[cfg(test)]
mod tests;
