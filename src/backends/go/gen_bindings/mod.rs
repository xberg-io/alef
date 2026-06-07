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
        // Derive the FFI crate directory from the configured output path.
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
        // Collect bridge param names from trait_bridges config so we can strip them
        // from generated function signatures and emit ConvertWithVisitor instead.
        let bridge_param_names: HashSet<String> = config
            .trait_bridges
            .iter()
            .filter_map(|b| b.param_name.clone())
            .collect();
        // Also collect type aliases used as bridge params (e.g. "VisitorHandle").
        let bridge_type_aliases: HashSet<String> = config
            .trait_bridges
            .iter()
            .filter_map(|b| b.type_alias.clone())
            .collect();
        // Determine if any bridge is configured for the visitor pattern.
        // Options-field bridges generate visitor.go regardless of visitor_callbacks.
        let visitor_callbacks_enabled = config.ffi.as_ref().is_some_and(|f| f.visitor_callbacks);
        let visitor_bridge_cfg = config
            .trait_bridges
            .iter()
            .find(|b| b.bind_via == BridgeBinding::OptionsField);
        let has_options_field_bridge = visitor_bridge_cfg.is_some();
        let has_visitor_bridge =
            has_options_field_bridge || (!config.trait_bridges.is_empty() && visitor_callbacks_enabled);

        // Determine if any plugin-style bridges (with register_fn) are configured.
        // These are independent of visitor_callbacks and generate trait_bridges.go.
        let has_plugin_bridges = config.trait_bridges.iter().any(|b| b.register_fn.is_some());

        // Map streaming adapter (owner_type, method_name) → item_type. The callback-based
        // FFI export (`<prefix>_<type>_<method>`) cannot be driven from CGO, but the
        // companion iterator-handle exports (`_start`, `_next`, `_free`) can — we emit a
        // dedicated Go method that drives them and returns a typed channel.
        // Adapters missing `owner_type` or `item_type` are skipped (treated as "no Go
        // streaming method emitted") rather than producing broken code.
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

        // Collect functions excluded from FFI generation. Go bindings call C symbols directly
        // via cgo, so any function excluded from the FFI header must also be excluded here.
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
        // Extend exclude_types with types marked as binding_excluded in the IR.
        exclude_types.extend(api.types.iter().filter(|t| t.binding_excluded).map(|t| t.name.clone()));
        // Mirror the FFI backend's `contains('<')` filter for declared opaque types: a
        // workspace-declared opaque whose `rust_path` carries generic parameters (e.g.
        // `axum::http::Request<axum::body::Body>`) cannot be represented in the C ABI,
        // so the FFI backend does not emit `_new`/`_free` symbols for it. The Go cgo
        // shim references those symbols, so the Go backend must follow the same rule —
        // otherwise generated Go code references `C.{prefix}_request_free` which the
        // linker cannot resolve.
        exclude_types.extend(
            config
                .opaque_types
                .iter()
                .filter(|(_, path)| path.contains('<'))
                .map(|(name, _)| name.clone()),
        );

        // Collect value-only types (all fields are primitives). These don't have _to_json
        // functions emitted by the FFI backend, so Go codegen must construct them from
        // field accessors instead of JSON deserialization.
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

        // Build adapter body map (consumed by generators via body substitution)
        let _adapter_bodies = crate::adapters::build_adapter_bodies(config, Language::Go)?;

        // Compute relative path from Go output dir to project root.
        let depth = output_dir.trim_end_matches('/').matches('/').count() + 1;
        let to_root = "../".repeat(depth);

        let mut files = vec![GeneratedFile {
            path: PathBuf::from(format!("{output_dir}binding.go")),
            content,
            generated_header: true,
        }];

        // Generate visitor.go when an options-field visitor bridge is configured.
        if has_visitor_bridge && let Some(bridge_cfg) = visitor_bridge_cfg {
            // Derive vtable_trait_name and options_field from the first options-field bridge,
            // which is the only bridge shape that can attach a visitor to an options DTO.
            let Some(options_field) = bridge_cfg.resolved_options_field() else {
                return Err(crate::core::AlefError::Config(
                    "Go visitor generation requires trait bridge options_field metadata".to_string(),
                )
                .into());
            };
            let vtable_trait_name = bridge_cfg.trait_name.clone();
            let options_field = options_field.to_string();

            // Look up the visitor trait def in the IR.
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

        // Generate trait_bridges.go for plugin-style bridges (with register_fn).
        // Per-call bridges (no register_fn) use visitor.go callbacks via convert() instead.
        // This is independent of visitor_callbacks, which only affects per-call bridges.
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

        // Generate the download tool under cmd/download_ffi/main.go
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
        // are included when this module is vendored. Go's go mod vendor command only
        // includes files that are referenced in the module; this directive tells Go
        // to include the include/ directory so that the cgo #include directives work
        // in vendored environments.
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
        // Go's binding.go IS the public API — no additional wrapper needed.
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
