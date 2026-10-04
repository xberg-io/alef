use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, EnumDef, EnumVariant, FieldDef, FunctionDef, ParamDef, TypeRef};
use std::path::{Path, PathBuf};

/// ~keep A cold full-release build can exceed two minutes after dependency upgrades, so real
/// toolchain probes get a bounded five-minute cold-start budget. The persistent cache below makes
/// subsequent cases incremental.
pub(super) const HOST_ENUM_CARGO_PROBE_TIMEOUT_SECS: u64 = 300;

/// ~keep Cargo fingerprints path dependencies by source path, so the fixture workspace and target
/// directory must both be stable. The real-Cargo guard serializes fixture rewrites and keeps this
/// checkout-local cache isolated from every other worktree.
pub(super) struct HostEnumCargoProbe {
    _cargo_guard: crate::test_support::RealCargoGuard,
    root: PathBuf,
    target_dir: PathBuf,
}

impl HostEnumCargoProbe {
    pub(super) fn acquire() -> Self {
        let cargo_guard = crate::test_support::RealCargoGuard::acquire();
        let (root, target_dir) = host_enum_cargo_probe_paths();
        std::fs::create_dir_all(&root).expect("host enum probe workspace");
        std::fs::create_dir_all(&target_dir).expect("host enum probe target directory");
        Self {
            _cargo_guard: cargo_guard,
            root,
            target_dir,
        }
    }

    pub(super) fn root(&self) -> &Path {
        &self.root
    }

    pub(super) fn command(&self, subcommand: &str) -> std::process::Command {
        let mut command = crate::test_support::spawn_from_stable_dir("cargo");
        command
            .current_dir(&self.root)
            .arg(subcommand)
            .args(["--offline", "--quiet", "--manifest-path"])
            .arg(self.root.join("crates/core-lib-php/Cargo.toml"))
            .env("CARGO_TARGET_DIR", &self.target_dir)
            .env("CARGO_BUILD_JOBS", "1");
        command
    }
}

fn host_enum_cargo_probe_paths() -> (PathBuf, PathBuf) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("php-host-enum-feature-probes");
    (base.join("workspace"), base.join("cargo-target"))
}

pub(super) fn assert_probe_succeeded(success: bool, output: &str) {
    assert!(success, "{output}");
}

pub(super) fn surface(shared_function: bool) -> ApiSurface {
    ApiSurface {
        crate_name: "core-lib".into(),
        version: "0.1.0".into(),
        enums: vec![
            EnumDef {
                name: "Backend".into(),
                rust_path: "core_lib::Backend".into(),
                serde_tag: Some("kind".into()),
                variants: vec![
                    EnumVariant {
                        name: "Memory".into(),
                        is_default: true,
                        ..Default::default()
                    },
                    EnumVariant {
                        name: "Remote".into(),
                        cfg: Some(r#"feature = "remote""#.into()),
                        fields: vec![FieldDef {
                            name: "endpoint".into(),
                            ty: TypeRef::String,
                            ..Default::default()
                        }],
                        ..Default::default()
                    },
                    EnumVariant {
                        name: "RemoteUnit".into(),
                        cfg: Some(r#"feature = "remote""#.into()),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
            EnumDef {
                name: "Mode".into(),
                rust_path: "core_lib::Mode".into(),
                variants: vec![
                    EnumVariant {
                        name: "Local".into(),
                        is_default: true,
                        ..Default::default()
                    },
                    EnumVariant {
                        name: "Remote".into(),
                        cfg: Some(r#"feature = "remote""#.into()),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
        ],
        functions: if shared_function {
            vec![FunctionDef {
                params: vec![ParamDef {
                    name: "mode".into(),
                    ty: TypeRef::Named("Mode".into()),
                    ..Default::default()
                }],
                name: "remote_enabled".into(),
                rust_path: "core_lib::remote_enabled".into(),
                cfg: Some(r#"feature = "remote""#.into()),
                ..Default::default()
            }]
        } else {
            vec![]
        },
        ..Default::default()
    }
}

pub(super) fn config(root: &Path, enabled: bool) -> ResolvedCrateConfig {
    let options = if enabled {
        "features = [\"full\"]"
    } else {
        "excluded_default_features = [\"remote\"]"
    };
    let source = format!(
        "[workspace]\nlanguages=[\"php\"]\n[[crates]]\nname=\"core-lib\"\nsources=[\"crates/core-lib/src/lib.rs\"]\n[crates.php]\n{options}\n"
    );
    let cfg: NewAlefConfig = toml::from_str(&source).expect("valid config");
    let mut config = cfg.resolve().expect("resolve config").remove(0);
    config.workspace_root = Some(root.to_path_buf());
    config
}

#[test]
fn shared_function_feature_remains_publicly_forwarded() {
    let features = crate::scaffold::languages::php::php_declared_features(&surface(true), &[], &[]);
    assert_eq!(features, std::collections::BTreeSet::from(["remote".to_string()]));
}

#[test]
fn function_only_feature_remains_publicly_forwarded() {
    let mut api = surface(true);
    api.enums.clear();
    assert_eq!(
        crate::scaffold::languages::php::php_declared_features(&api, &[], &[]),
        std::collections::BTreeSet::from(["remote".to_string()])
    );
}

#[test]
fn configured_wrapper_default_remains_in_php_declared_feature_namespace() {
    let api = ApiSurface::default();
    let configured = vec!["formula-recognition".to_string()];

    assert_eq!(
        crate::scaffold::languages::php::php_declared_features(&api, &configured, &[]),
        std::collections::BTreeSet::from(["formula-recognition".to_string()])
    );
}

pub(super) fn write_fixture(root: &Path, enabled: bool, target: Option<bool>, opt_in: bool, aggregate: bool) {
    let core = root.join("crates/core-lib");
    let binding = root.join("crates/core-lib-php");
    std::fs::create_dir_all(core.join("src")).expect("core directory");
    std::fs::create_dir_all(binding.join("src")).expect("binding directory");
    std::fs::write(
        core.join("Cargo.toml"),
        "[package]\nname=\"core-lib\"\nversion=\"0.1.0\"\nedition=\"2024\"\n[features]\nfull=[\"remote\"]\nremote=[]\nextra=[\"remote\"]\n",
    )
    .expect("core manifest");
    std::fs::write(
        core.join("src/lib.rs"),
        r#"
#[derive(Debug, Default)]
pub enum Backend { #[default] Memory, #[cfg(feature="remote")] Remote { endpoint: String }, #[cfg(feature="remote")] RemoteUnit, #[cfg(feature="extra")] Aggregate }
"#,
    )
    .expect("core source");
    let mut api = surface((enabled && target.is_none()) || aggregate);
    if aggregate {
        api.enums[0].variants.push(EnumVariant {
            name: "Aggregate".into(),
            cfg: Some(r#"feature = "extra""#.into()),
            ..Default::default()
        });
    }
    let mut config = config(root, enabled);
    if let Some(target_enabled) = target {
        config
            .php
            .as_mut()
            .expect("PHP config")
            .target_dep_overrides
            .push(crate::core::config::FfiTargetDepOverride {
                cfg: format!("target_os = \"{}\"", std::env::consts::OS),
                features: if target_enabled { vec!["remote".into()] } else { vec![] },
                default_features: false,
            });
    }
    let files = crate::scaffold::languages::php::scaffold_php_cargo(&api, &config).expect("scaffold");
    let manifest = files
        .iter()
        .find(|file| file.path.ends_with("Cargo.toml"))
        .expect("manifest");
    let mut manifest: toml::Value = toml::from_str(&manifest.content).expect("generated TOML");
    manifest["dependencies"]
        .as_table_mut()
        .expect("dependencies")
        .retain(|name, _| name == "core-lib");
    let root = manifest.as_table_mut().expect("manifest table");
    root.remove("lib");
    root.insert("workspace".into(), toml::Value::Table(toml::Table::new()));
    std::fs::write(
        binding.join("Cargo.toml"),
        toml::to_string(&manifest).expect("fixture manifest"),
    )
    .expect("write manifest");
    use crate::core::backend::Backend;
    let generated = super::PhpBackend.generate_bindings(&api, &config).expect("bindings");
    let source = &generated
        .iter()
        .find(|file| file.path.ends_with("lib.rs"))
        .expect("lib.rs")
        .content;
    let mut conversion = String::new();
    for marker in [
        "impl From<core_lib::Backend> for Backend",
        "impl From<Backend> for core_lib::Backend",
    ] {
        let start = source.find(marker).expect("conversion start");
        let end = source[start..].find("\n}").expect("conversion end") + start + 2;
        conversion.push_str(&source[start..end]);
        conversion.push('\n');
    }
    let exercise = if (aggregate && opt_in) || target.unwrap_or(enabled || opt_in) {
        r#"let unit = Backend::from(core_lib::Backend::RemoteUnit); assert_eq!(unit.kind_tag, "RemoteUnit"); assert!(matches!(core_lib::Backend::from(unit), core_lib::Backend::RemoteUnit)); let value = Backend::from(core_lib::Backend::Remote { endpoint: "identity".into() }); assert_eq!(value.endpoint.as_deref(), Some("identity")); assert!(matches!(core_lib::Backend::from(value), core_lib::Backend::Remote { endpoint } if endpoint == "identity"));"#
    } else {
        r#"let value = Backend::from(core_lib::Backend::Memory); assert_eq!(value.kind_tag, "Memory"); assert!(matches!(core_lib::Backend::from(value), core_lib::Backend::Memory));"#
    };
    let source = format!(
        "#[derive(Default)]\nstruct Backend {{ kind_tag: String, endpoint: Option<String> }}\n{conversion}\nfn main() {{ {exercise} }}\n"
    );
    std::fs::write(binding.join("src/main.rs"), source).expect("binding source");
}

fn compile_fixture(enabled: bool, no_defaults: bool, target: Option<bool>, opt_in: bool, aggregate: bool) {
    let probe = HostEnumCargoProbe::acquire();
    write_fixture(probe.root(), enabled, target, opt_in, aggregate);
    let mut command = probe.command("run");
    command.env("RUSTFLAGS", "-Dunexpected_cfgs");
    if no_defaults {
        command.arg("--no-default-features");
    }
    if opt_in {
        command.args(["--features", if aggregate { "extra" } else { "remote" }]);
    }
    let (success, output) = crate::snippets::validators::run_command(&mut command, HOST_ENUM_CARGO_PROBE_TIMEOUT_SECS)
        .expect("compile and execute generated conversion within deadline");
    assert_probe_succeeded(success, &output);
}

#[test]
fn emitted_conversion_compiles_with_transitive_full_feature() {
    compile_fixture(true, false, None, false, false);
}

#[test]
fn emitted_conversion_compiles_with_optional_feature_disabled() {
    compile_fixture(false, true, None, false, false);
}

#[test]
fn emitted_conversion_preserves_forced_core_feature_without_binding_defaults() {
    compile_fixture(true, true, None, false, false);
}

#[test]
fn emitted_conversion_supports_optional_feature_opt_in() {
    compile_fixture(false, true, None, true, false);
}

#[test]
fn emitted_conversion_respects_target_override_disabling_core_feature() {
    compile_fixture(true, true, Some(false), false, false);
}

#[test]
fn emitted_conversion_respects_target_override_enabling_core_feature() {
    compile_fixture(false, true, Some(true), false, false);
}

#[test]
fn aggregate_passthrough_enables_transitive_core_variant_on_overridden_target() {
    compile_fixture(true, true, Some(false), true, true);
}

#[test]
fn cargo_probes_share_a_checkout_scoped_workspace_and_target() {
    let first = host_enum_cargo_probe_paths();
    let second = host_enum_cargo_probe_paths();
    let expected_base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("php-host-enum-feature-probes");

    assert_eq!(first, second);
    assert_eq!(first.0, expected_base.join("workspace"));
    assert_eq!(first.1, expected_base.join("cargo-target"));
}

#[test]
fn cargo_probe_command_uses_the_shared_target_and_bounded_budget() {
    let probe = HostEnumCargoProbe::acquire();
    write_fixture(probe.root(), true, None, false, false);
    let command = probe.command("run");
    let target_dir = command
        .get_envs()
        .find_map(|(key, value)| (key == "CARGO_TARGET_DIR").then_some(value))
        .flatten();

    assert_eq!(target_dir, Some(probe.target_dir.as_os_str()));
    assert_eq!(command.get_current_dir(), Some(probe.root()));
    assert_eq!(HOST_ENUM_CARGO_PROBE_TIMEOUT_SECS, 300);
    let manifest =
        std::fs::read_to_string(probe.root().join("crates/core-lib-php/Cargo.toml")).expect("fixture manifest");
    let manifest: toml::Value = toml::from_str(&manifest).expect("fixture manifest TOML");
    let workspace = manifest
        .get("workspace")
        .and_then(toml::Value::as_table)
        .expect("fixture must be its own workspace root");
    assert!(workspace.is_empty(), "{workspace:?}");
}

#[test]
fn failed_cargo_probe_output_remains_a_test_failure() {
    let result = std::panic::catch_unwind(|| assert_probe_succeeded(false, "fixture compile failed"));
    let panic = result.expect_err("failed subprocess must fail the test");
    let message = panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&str>().copied())
        .expect("panic payload");

    assert!(message.contains("fixture compile failed"), "{message}");
}
