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
    let content =
        crate::backends::pyo3::gen_stubs::gen_stubs(api, &config.trait_bridges, config, &stubs_exclude_functions);

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

    // Use stubs output path as the package directory (e.g., packages/python/sample_markdown/)
    // This ensures we write to the correct Python package, not the Rust crate name.
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

    // Types re-exported in the public package as native pyclasses (e.g. `ExtractionResult`). Such a
    // type is native everywhere — it must NOT also be emitted as a parallel `options.py` TypedDict,
    // or fields referencing it would carry a structurally-incompatible second identity.
    let reexported_types = config
        .python
        .as_ref()
        .map(|c| c.reexported_types.clone())
        .unwrap_or_default();

    // 1. Generate options.py (enums and dataclasses)
    let options_content = types::gen_options_py(api, &module_name, &config.dto, &reexported_types);
    files.push(GeneratedFile {
        path: output_base.join("options.py"),
        content: options_content,
        generated_header: true,
    });

    // 2. Generate api.py (wrapper functions)
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

    // 3. Generate exceptions.py (exception hierarchy)
    let exceptions_content = errors::gen_exceptions_py(api, &module_name);
    files.push(GeneratedFile {
        path: output_base.join("exceptions.py"),
        content: exceptions_content,
        generated_header: true,
    });

    // 4. Generate __init__.py (re-exports)
    let extra_init_imports = config
        .python
        .as_ref()
        .map(|c| c.extra_init_imports.clone())
        .unwrap_or_default();
    let init_content = errors::gen_init_py(
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
    files.push(GeneratedFile {
        path: output_base.join("__init__.py"),
        content: init_content,
        generated_header: true,
    });

    Ok(files)
}
