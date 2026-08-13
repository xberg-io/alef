use super::{errors, functions, types};
use crate::core::backend::GeneratedFile;
use crate::core::config::{ResolvedCrateConfig, resolve_output_dir};
use crate::core::ir::ApiSurface;
use ahash::AHashSet;
use std::path::PathBuf;

pub(super) fn generate_type_stubs(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
) -> anyhow::Result<Vec<GeneratedFile>> {
    let stubs_config = match config.python.as_ref().and_then(|c| c.stubs.as_ref()) {
        Some(s) => s,
        None => return Ok(vec![]),
    };

    let stubs_exclude_functions: AHashSet<String> = config
        .python
        .as_ref()
        .map(|c| c.exclude_functions.iter().cloned().collect())
        .unwrap_or_default();
    let mut content =
        crate::backends::pyo3::gen_stubs::gen_stubs(api, &config.trait_bridges, config, &stubs_exclude_functions);
    if !config.components.is_empty() {
        content.push_str(
            "\n\ndef component_load(component: str) -> None: ...\n\ndef component_prefetch(components: list[str] | None = None) -> list[str]: ...\n\ndef component_status(component: str) -> str: ...\n\ndef component_cache_path(component: str) -> str: ...\n",
        );
    }

    let stubs_path = resolve_output_dir(
        Some(&stubs_config.output),
        &config.name,
        stubs_config.output.to_string_lossy().as_ref(),
    );

    Ok(vec![GeneratedFile {
        path: PathBuf::from(&stubs_path).join(format!("{}.pyi", config.python_module_name())),
        content,
        generated_header: true,
    }])
}

pub(super) fn generate_public_api(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
) -> anyhow::Result<Vec<GeneratedFile>> {
    let module_name = config.python_module_name();

    let output_base = config
        .python
        .as_ref()
        .and_then(|p| p.stubs.as_ref())
        .map(|s| PathBuf::from(&s.output))
        .unwrap_or_else(|| {
            let package_name = config.name.replace('-', "_");
            PathBuf::from(format!("packages/python/{}", package_name))
        });
    let package_name = output_base
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| config.name.replace('-', "_"));

    let mut files = vec![];

    let reexported_types = config
        .python
        .as_ref()
        .map(|c| c.reexported_types.clone())
        .unwrap_or_default();

    let options_content = types::gen_options_py(api, &module_name, &config.dto, &reexported_types);
    files.push(GeneratedFile {
        path: output_base.join("options.py"),
        content: options_content,
        generated_header: true,
    });

    let exclude_functions: AHashSet<String> = config
        .python
        .as_ref()
        .map(|c| c.exclude_functions.iter().cloned().collect())
        .unwrap_or_default();
    let capsule_types = config
        .python
        .as_ref()
        .map(|c| c.capsule_types.clone())
        .unwrap_or_default();
    let api_content = functions::gen_api_py(
        api,
        &module_name,
        &package_name,
        &config.trait_bridges,
        &config.dto,
        &capsule_types,
        &config.opaque_types,
        &config.adapters,
        &reexported_types,
        &exclude_functions,
    );
    files.push(GeneratedFile {
        path: output_base.join("api.py"),
        content: api_content,
        generated_header: true,
    });

    let exceptions_content = errors::gen_exceptions_py(api, &module_name);
    files.push(GeneratedFile {
        path: output_base.join("exceptions.py"),
        content: exceptions_content,
        generated_header: true,
    });

    let extra_init_imports = config
        .python
        .as_ref()
        .map(|c| c.extra_init_imports.clone())
        .unwrap_or_default();
    let mut init_content = errors::gen_init_py(
        api,
        &module_name,
        &api.version,
        &config.dto,
        &reexported_types,
        &config.trait_bridges,
        &extra_init_imports,
        &capsule_types,
        &config.adapters,
        &config.opaque_types,
        &exclude_functions,
    );
    if !config.components.is_empty() {
        let component_names = [
            "component_cache_path",
            "component_load",
            "component_prefetch",
            "component_status",
        ];
        init_content.push_str("\nfrom .components import (\n");
        for name in component_names {
            init_content.push_str(&format!("    {name},\n"));
        }
        init_content.push_str(")\n\n__all__.extend([\n");
        for name in component_names {
            init_content.push_str(&format!("    \"{name}\",\n"));
        }
        init_content.push_str("])\n");

        let component_content = format!(
            "from .{module_name} import (\n    component_cache_path,\n    component_load,\n    component_prefetch,\n    component_status,\n)\n\n__all__ = [\n    \"component_cache_path\",\n    \"component_load\",\n    \"component_prefetch\",\n    \"component_status\",\n]\n"
        );
        files.push(GeneratedFile {
            path: output_base.join("components.py"),
            content: component_content,
            generated_header: true,
        });
    }
    files.push(GeneratedFile {
        path: output_base.join("__init__.py"),
        content: init_content,
        generated_header: true,
    });

    Ok(files)
}
