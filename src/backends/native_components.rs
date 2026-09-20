//! Shared Rust-side component manager emitted by native binding backends.

use crate::codegen::component::{ResolvedComponent, entry_point_symbol, resolve_components};
use crate::codegen::component_proxy::generate_component_proxies;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::ApiSurface;
use anyhow::{Context as _, Result};
use heck::{ToShoutySnakeCase, ToUpperCamelCase};
use std::fmt::Write as _;

/// Generate backend-neutral helpers around `alef-component-runtime`.
///
/// `lock_manifest_path` is relative to the generated binding crate's
/// `CARGO_MANIFEST_DIR` and must start with `/` so it can be passed directly to
/// `concat!`.
pub(crate) fn generate(config: &ResolvedCrateConfig, lock_manifest_path: &str) -> String {
    debug_assert!(lock_manifest_path.starts_with('/'));
    let cache_env = format!("{}_COMPONENT_CACHE", config.name.to_shouty_snake_case());
    let offline_env = format!("{}_COMPONENT_OFFLINE", config.name.to_shouty_snake_case());
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
            .map(|manager| manager.offline(std::env::var_os("{offline_env}").is_some()))
            .map_err(|error| error.to_string())
        }})
        .as_ref()
        .map_err(Clone::clone)
}}

/// Download and verify a configured component's artifact, without loading or
/// registering any of the contracts it provides. A component now provides one
/// or more contracts, each through its own producer entry point, so
/// "load the component" no longer names a single, well-defined dlopen the way
/// it did for a single-contract component; call `alef_component_activate`
/// (`component_activate`/`ComponentActivate`) to actually load and register.
fn alef_component_load(component: &str) -> Result<(), String> {{
    alef_component_manager()?
        .prefetch(&[component])
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

/// Generate the host-side glue that turns a loaded component into registered
/// contract providers: the per-contract proxies
/// ([`crate::codegen::component_proxy`]), the activation dispatch that builds
/// and registers one for every contract a component provides, and an
/// idempotent `alef_component_activate` entry point each backend exposes as
/// an explicit `component_activate`/`ComponentActivate` binding function.
///
/// Activation is deliberately not wired into any binding's module init: doing
/// so would make importing a generated module attempt component downloads.
/// Callers activate components explicitly, the same way they already
/// prefetch or load them.
///
/// Returns an empty string when the crate has no components, so callers can
/// unconditionally append the result.
pub(crate) fn generate_activation(api: &ApiSurface, config: &ResolvedCrateConfig) -> Result<String> {
    if config.components.is_empty() {
        return Ok(String::new());
    }
    let resolved = resolve_components(api, config)?;
    let mut out = generate_component_proxies(config, &resolved)?;
    write!(out, "{}", activation_dispatch(config, &resolved)?)?;
    out.push_str(ACTIVATION_SUPPORT);
    Ok(out)
}

/// One `manager.ensure_contract` + proxy-construction + `register_provider`
/// block per contract a component provides. `entry_point_symbol` is computed
/// here, at generation time, and baked in as a byte-string literal, because
/// computing it correctly requires the same identifier-casing logic the
/// producer used (`heck`), which `alef-component-runtime` does not depend on.
fn activation_dispatch(config: &ResolvedCrateConfig, resolved: &[ResolvedComponent]) -> Result<String> {
    let mut out = String::new();
    out.push_str(
        "fn alef_component_register_providers(component: &str) -> Result<(), String> {\n    let manager = alef_component_manager()?;\n    match component {\n",
    );
    for component in resolved {
        writeln!(out, "        {:?} => {{", component.component_name)?;
        for provided in &component.provides {
            let contract_cfg = config
                .component_contracts
                .iter()
                .find(|candidate| candidate.name == provided.contract.name)
                .with_context(|| {
                    format!(
                        "resolved contract `{}` has no matching config entry",
                        provided.contract.name
                    )
                })?;
            let proxy_name = format!("{}Proxy", provided.contract.name.to_upper_camel_case());
            let entry_symbol = entry_point_symbol(&provided.contract.name);
            writeln!(
                out,
                "            let loaded = manager.ensure_contract({component:?}, {contract:?}, b\"{entry_symbol}\\0\").map_err(|error| error.to_string())?;",
                component = component.component_name,
                contract = provided.contract.name,
            )?;
            writeln!(
                out,
                "            let proxy = std::sync::Arc::new({proxy_name}::new(&loaded)?);"
            )?;
            writeln!(
                out,
                "            alef_component_abi::register_provider::<dyn {}>({:?}, proxy);",
                contract_cfg.trait_path, provided.contract.name
            )?;
        }
        out.push_str("            Ok(())\n        }\n");
    }
    out.push_str("        other => Err(format!(\"component `{other}` has no activation wiring\")),\n");
    out.push_str("    }\n}\n\n");
    Ok(out)
}

const ACTIVATION_SUPPORT: &str = r#"static ALEF_COMPONENT_ACTIVATED: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> =
    std::sync::OnceLock::new();

fn alef_component_activated() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
    ALEF_COMPONENT_ACTIVATED.get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()))
}

/// Ensure `component` is downloaded, loaded, and its contracts registered with
/// `alef_component_abi`. Safe to call more than once; only the first call for
/// a given component does the work.
fn alef_component_activate(component: &str) -> Result<(), String> {
    let mut activated = alef_component_activated()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if activated.contains(component) {
        return Ok(());
    }
    alef_component_register_providers(component)?;
    activated.insert(component.to_string());
    Ok(())
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::{ComponentConfig, ComponentContractConfig, ComponentProvidesConfig};
    use crate::core::ir::{MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};

    fn codec_fixture() -> (ApiSurface, ResolvedCrateConfig) {
        let api = ApiSurface {
            crate_name: "demo_core".into(),
            version: "1.0.0".into(),
            types: vec![TypeDef {
                name: "Codec".into(),
                rust_path: "demo_core::Codec".into(),
                is_trait: true,
                is_opaque: true,
                methods: vec![MethodDef {
                    name: "transform".into(),
                    params: vec![ParamDef {
                        name: "input".into(),
                        ty: TypeRef::Bytes,
                        is_ref: true,
                        ..ParamDef::default()
                    }],
                    return_type: TypeRef::Bytes,
                    receiver: Some(ReceiverKind::Ref),
                    error_type: Some("Error".into()),
                    ..MethodDef::default()
                }],
                ..TypeDef::default()
            }],
            ..ApiSurface::default()
        };
        let config = ResolvedCrateConfig {
            name: "demo-core".into(),
            component_contracts: vec![ComponentContractConfig {
                name: "codec".into(),
                trait_path: "demo_core::Codec".into(),
                interface_version: 1,
            }],
            components: vec![ComponentConfig {
                name: "reverse".into(),
                provides: vec![ComponentProvidesConfig {
                    contract: "codec".into(),
                    implementation: "demo_core::ReverseCodec".into(),
                }],
                features: vec!["reverse".into()],
                default_features: false,
                targets: Some(vec!["x86_64-unknown-linux-gnu".into()]),
                bundled_on: Vec::new(),
            }],
            ..ResolvedCrateConfig::default()
        };
        (api, config)
    }

    #[test]
    fn activation_registers_every_contract_a_component_provides() {
        let (api, config) = codec_fixture();
        let generated = generate_activation(&api, &config).unwrap();
        assert!(generated.contains("pub struct CodecProxy"), "{generated}");
        assert!(generated.contains("\"reverse\" => {"), "{generated}");
        assert!(
            generated
                .contains("manager.ensure_contract(\"reverse\", \"codec\", b\"alef_component_entry_v1_codec\\0\")"),
            "{generated}"
        );
        assert!(
            generated.contains("register_provider::<dyn demo_core::Codec>(\"codec\", proxy)"),
            "{generated}"
        );
        assert!(
            generated.contains("fn alef_component_activate(component: &str)"),
            "{generated}"
        );

        let wrapped = format!("mod generated {{ {generated} }}");
        syn::parse_file(&wrapped).expect("generated activation glue must be valid Rust syntax");
    }

    #[test]
    fn activation_is_empty_without_configured_components() {
        let config = ResolvedCrateConfig {
            name: "demo-core".into(),
            ..ResolvedCrateConfig::default()
        };
        let api = ApiSurface::default();
        assert_eq!(generate_activation(&api, &config).unwrap(), "");
    }

    #[test]
    fn generates_embedded_lock_and_all_management_operations() {
        let config = ResolvedCrateConfig {
            name: "demo-core".into(),
            component_contracts: vec![ComponentContractConfig {
                name: "engine".into(),
                trait_path: "demo_core::Engine".into(),
                interface_version: 1,
            }],
            components: vec![ComponentConfig {
                name: "fast".into(),
                provides: vec![ComponentProvidesConfig {
                    contract: "engine".into(),
                    implementation: "demo_core::FastEngine".into(),
                }],
                features: vec!["fast".into()],
                default_features: false,
                targets: Some(vec!["x86_64-unknown-linux-gnu".into()]),
                bundled_on: Vec::new(),
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

    #[test]
    fn generated_manager_honors_the_crate_specific_offline_env_var() {
        let config = ResolvedCrateConfig {
            name: "demo-core".into(),
            ..ResolvedCrateConfig::default()
        };
        let generated = generate(&config, "/components.lock.json");
        assert!(
            generated.contains(r#"manager.offline(std::env::var_os("DEMO_CORE_COMPONENT_OFFLINE").is_some())"#),
            "{generated}"
        );
    }
}
