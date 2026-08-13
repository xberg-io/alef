use super::{cfg_registration, options, r_wrappers};
use crate::core::backend::GeneratedFile;
use crate::core::config::{ResolvedCrateConfig, resolve_output_dir};
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::ApiSurface;
use std::path::PathBuf;

pub(super) fn generate_public_api(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
) -> anyhow::Result<Vec<GeneratedFile>> {
    let enabled_features = cfg_registration::effective_r_cfg_features(api, config);
    let r_cfg_api = cfg_registration::apply_r_cfg_field_policy(api, &enabled_features);
    let api = &r_cfg_api;
    let package_name = config.r_package_name();

    let r_wrapper_dir = if let Some(rust_out) = config.output_paths.get("r") {
        let rust_str = rust_out.to_string_lossy();
        let suffixes = ["src/rust/src/", "src/rust/src"];
        let base = suffixes
            .iter()
            .find_map(|s| rust_str.strip_suffix(s))
            .unwrap_or_else(|| rust_str.as_ref());
        format!("{base}R/")
    } else {
        "packages/r/R/".to_string()
    };
    let r_pkg_dir = r_wrapper_dir.trim_end_matches("R/").trim_end_matches("R");

    let mut files = Vec::new();

    let mut pkg_content = hash::header(CommentStyle::Hash);
    pkg_content.push('\n');
    pkg_content.push_str(&crate::backends::extendr::template_env::render(
        "r_use_dyn_lib.jinja",
        minijinja::context! { package_name => package_name },
    ));
    pkg_content.push_str("NULL\n");
    files.push(GeneratedFile {
        path: PathBuf::from(&r_wrapper_dir).join(format!("{package_name}.R")),
        content: pkg_content,
        generated_header: false,
    });

    let input_type_names = crate::codegen::conversions::input_type_names(api);
    let trait_bridge_fns = super::trait_bridge_wrappers::collect_trait_bridge_functions(config);
    let r_exclude_functions: ahash::AHashSet<String> = config
        .r
        .as_ref()
        .map(|c| c.exclude_functions.iter().cloned().collect())
        .unwrap_or_default();
    let mut wrappers_content = r_wrappers::gen_extendr_wrappers_r(
        api,
        &package_name,
        &input_type_names,
        &trait_bridge_fns,
        &r_exclude_functions,
        &config.trait_bridges,
    );
    if !config.components.is_empty() {
        wrappers_content.push_str(&format!(
            "#' Load a downloadable native component\n#' @param component Component identifier.\n#' @export\ncomponent_load <- function(component) .Call(\"wrap__component_load\", component, PACKAGE = \"{package_name}\")\n\n#' Prefetch downloadable native components\n#' @param components Component identifiers, or NULL for all configured components.\n#' @export\ncomponent_prefetch <- function(components = NULL) .Call(\"wrap__component_prefetch\", components, PACKAGE = \"{package_name}\")\n\n#' Inspect a downloadable native component\n#' @param component Component identifier.\n#' @export\ncomponent_status <- function(component) .Call(\"wrap__component_status\", component, PACKAGE = \"{package_name}\")\n\n#' Return a downloadable native component cache path\n#' @param component Component identifier.\n#' @export\ncomponent_cache_path <- function(component) .Call(\"wrap__component_cache_path\", component, PACKAGE = \"{package_name}\")\n\n"
        ));
    }
    files.push(GeneratedFile {
        path: PathBuf::from(&r_wrapper_dir).join("extendr-wrappers.R"),
        content: wrappers_content,
        generated_header: false,
    });

    let mut namespace_content = r_wrappers::gen_namespace(
        api,
        &package_name,
        &trait_bridge_fns,
        &r_exclude_functions,
        &config.trait_bridges,
    );
    if !config.components.is_empty() {
        namespace_content.push_str(
            "export(component_load)\nexport(component_prefetch)\nexport(component_status)\nexport(component_cache_path)\n",
        );
    }
    files.push(GeneratedFile {
        path: PathBuf::from(r_pkg_dir).join("NAMESPACE"),
        content: namespace_content,
        generated_header: false,
    });

    if let Some(opts_type) = options::find_r_options_type(api, config) {
        files.push(GeneratedFile {
            path: PathBuf::from(&r_wrapper_dir).join("options.R"),
            content: options::gen_conversion_options_r(opts_type),
            generated_header: true,
        });
    }

    if let Some(opts_type) = options::find_r_options_type(api, config) {
        let core_import = config.core_import_name();
        let options_rs = options::gen_options_rs(api, opts_type, &core_import);
        let rust_output_path =
            resolve_output_dir(config.output_paths.get("r"), &config.name, "packages/r/src/rust/src");
        files.push(GeneratedFile {
            path: PathBuf::from(&rust_output_path).join("options.rs"),
            content: options_rs,
            generated_header: true,
        });
    }

    Ok(files)
}
