//! Shared Rust-side component manager emitted by native binding backends.

use crate::core::config::ResolvedCrateConfig;
use heck::ToShoutySnakeCase;

/// Generate backend-neutral helpers around `alef-component-runtime`.
///
/// `lock_manifest_path` is relative to the generated binding crate's
/// `CARGO_MANIFEST_DIR` and must start with `/` so it can be passed directly to
/// `concat!`.
pub(crate) fn generate(config: &ResolvedCrateConfig, lock_manifest_path: &str) -> String {
    debug_assert!(lock_manifest_path.starts_with('/'));
    let cache_env = format!("{}_COMPONENT_CACHE", config.name.to_shouty_snake_case());
    let cache_namespace = config.name.replace('-', "_");
    let component_ids = config
        .components
        .iter()
        .map(|component| serde_json::to_string(&component.name).expect("component name is serializable"))
        .collect::<Vec<_>>()
        .join(", ");

    format!(
        r#"static ALEF_COMPONENT_MANAGER: std::sync::OnceLock<Result<alef_component_runtime::ComponentManager, String>> = std::sync::OnceLock::new();

fn alef_component_target() -> Result<&'static str, String> {{
    #[cfg(all(target_arch = "x86_64", target_os = "linux", target_env = "gnu"))]
    {{ return Ok("x86_64-unknown-linux-gnu"); }}
    #[cfg(all(target_arch = "aarch64", target_os = "linux", target_env = "gnu"))]
    {{ return Ok("aarch64-unknown-linux-gnu"); }}
    #[cfg(all(target_arch = "x86_64", target_os = "macos"))]
    {{ return Ok("x86_64-apple-darwin"); }}
    #[cfg(all(target_arch = "aarch64", target_os = "macos"))]
    {{ return Ok("aarch64-apple-darwin"); }}
    #[cfg(all(target_arch = "x86_64", target_os = "windows", target_env = "msvc"))]
    {{ return Ok("x86_64-pc-windows-msvc"); }}
    #[allow(unreachable_code)]
    Err(format!(
        "downloadable native components are unsupported on this host ({{}}-{{}}); supported v1 hosts are x86_64/aarch64 Linux GNU, x86_64/aarch64 macOS, and x86_64 Windows MSVC",
        std::env::consts::ARCH,
        std::env::consts::OS,
    ))
}}

fn alef_component_cache_root() -> std::path::PathBuf {{
    if let Some(path) = std::env::var_os("{cache_env}") {{
        return path.into();
    }}
    let base = directories::BaseDirs::new()
        .map(|dirs| dirs.cache_dir().to_path_buf())
        .unwrap_or_else(std::env::temp_dir);
    base.join("{cache_namespace}").join("components")
}}

fn alef_component_manager() -> Result<&'static alef_component_runtime::ComponentManager, String> {{
    ALEF_COMPONENT_MANAGER
        .get_or_init(|| {{
            let lock = serde_json::from_str(include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "{lock_manifest_path}"
            )))
            .map_err(|error| format!("invalid embedded components.lock.json: {{error}}"))?;
            let host = alef_component_abi::AlefHostApiV1 {{
                struct_size: 0,
                abi_major: 0,
                abi_minor: 0,
                context: std::ptr::null_mut(),
                log: None,
            }};
            alef_component_runtime::ComponentManager::from_lock(
                lock,
                alef_component_cache_root(),
                alef_component_target()?,
                host,
            )
            .map_err(|error| error.to_string())
        }})
        .as_ref()
        .map_err(Clone::clone)
}}

fn alef_component_load(component: &str) -> Result<(), String> {{
    alef_component_manager()?
        .ensure(component)
        .map(|_| ())
        .map_err(|error| error.to_string())
}}

fn alef_component_prefetch(components: Option<Vec<String>>) -> Result<Vec<String>, String> {{
    let components = components.unwrap_or_else(|| {{
        vec![{component_ids}]
            .into_iter()
            .map(str::to_string)
            .collect()
    }});
    let borrowed = components.iter().map(String::as_str).collect::<Vec<_>>();
    alef_component_manager()?
        .prefetch(&borrowed)
        .map(|artifacts| {{
            artifacts
                .into_iter()
                .map(|artifact| artifact.root.display().to_string())
                .collect()
        }})
        .map_err(|error| error.to_string())
}}

fn alef_component_status(component: &str) -> Result<String, String> {{
    let status = alef_component_manager()?
        .status(component)
        .map_err(|error| error.to_string())?;
    Ok(match status {{
        alef_component_runtime::ComponentStatus::Missing => "missing".to_string(),
        alef_component_runtime::ComponentStatus::Cached(path) => format!("cached:{{}}", path.display()),
        alef_component_runtime::ComponentStatus::Loaded(path) => format!("loaded:{{}}", path.display()),
    }})
}}

fn alef_component_cache_path(component: &str) -> Result<String, String> {{
    alef_component_manager()?
        .cache_path(component)
        .map(|path| path.display().to_string())
        .map_err(|error| error.to_string())
}}"#,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::{ComponentContractConfig, ComponentProfileConfig};

    #[test]
    fn generates_embedded_lock_and_all_management_operations() {
        let config = ResolvedCrateConfig {
            name: "demo-core".into(),
            component_contracts: vec![ComponentContractConfig {
                name: "engine".into(),
                trait_path: "demo_core::Engine".into(),
                interface_version: 1,
            }],
            components: vec![ComponentProfileConfig {
                name: "fast".into(),
                contract: "engine".into(),
                implementation: "demo_core::FastEngine".into(),
                features: vec!["fast".into()],
                default_features: false,
                targets: vec!["x86_64-unknown-linux-gnu".into()],
            }],
            ..ResolvedCrateConfig::default()
        };

        let generated = generate(&config, "/../../../components.lock.json");
        assert!(generated.contains("/../../../components.lock.json"));
        assert!(generated.contains("DEMO_CORE_COMPONENT_CACHE"));
        assert!(generated.contains("fn alef_component_load"));
        assert!(generated.contains("fn alef_component_prefetch"));
        assert!(generated.contains("fn alef_component_status"));
        assert!(generated.contains("fn alef_component_cache_path"));
        assert!(generated.contains("vec![\"fast\"]"));
        assert!(generated.contains("fn alef_component_target() -> Result<&'static str, String>"));
        assert!(generated.contains("downloadable native components are unsupported"));
    }
}
