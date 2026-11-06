use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail, ensure};

use crate::component::artifact::dynamic_library_name;
use crate::core::config::{ComponentConfig, ComponentProducerLanguage, ResolvedCrateConfig};

pub(super) fn build_component(
    config: &ResolvedCrateConfig,
    profile: &ComponentConfig,
    rust_target: &str,
    debug: bool,
    dry_run: bool,
    shared_target_dir: Option<&Path>,
) -> Result<()> {
    if profile.producer_language == ComponentProducerLanguage::Executable {
        return build_executable(config, profile, rust_target, debug, dry_run, shared_target_dir);
    }
    let mut producer_manifest = producer_manifest_path(config, profile);
    if profile.producer_language == ComponentProducerLanguage::Go {
        producer_manifest.set_file_name("go.mod");
    }
    let mut command = component_build_command(
        config,
        profile,
        rust_target,
        debug,
        shared_target_dir,
        &producer_manifest,
    )?;
    tracing::info!(
        "Building component `{}` for `{}` (features: {})",
        profile.name,
        rust_target,
        if profile.features.is_empty() {
            "<none>".to_string()
        } else {
            profile.features.join(",")
        }
    );
    tracing::debug!("component build command: {command:?}");
    if dry_run {
        return Ok(());
    }
    let library = built_library_path(config, &profile.name, rust_target, debug, shared_target_dir);
    execute_component_build(&mut command, profile, rust_target, &library)?;
    ensure!(
        library.is_file(),
        "component build succeeded but did not produce {}",
        library.display()
    );
    Ok(())
}

pub(super) fn component_build_command(
    config: &ResolvedCrateConfig,
    profile: &ComponentConfig,
    rust_target: &str,
    debug: bool,
    shared_target_dir: Option<&Path>,
    producer_manifest: &Path,
) -> Result<Command> {
    ensure!(
        producer_manifest.is_file(),
        "component producer manifest does not exist: {}; run `alef scaffold` first",
        producer_manifest.display()
    );
    if profile.producer_language == ComponentProducerLanguage::Go {
        let library = built_library_path(config, &profile.name, rust_target, debug, shared_target_dir);
        return go_build_command(rust_target, producer_manifest, &library);
    }
    let mut command = Command::new("cargo");
    command
        .arg("build")
        .arg("--manifest-path")
        .arg(producer_manifest)
        .args(["--target", rust_target, "--target-dir"])
        .arg(component_target_dir(config, &profile.name, shared_target_dir));
    if !debug {
        command.arg("--release");
    }
    Ok(command)
}

fn go_build_command(rust_target: &str, producer_manifest: &Path, library: &Path) -> Result<Command> {
    let (os, arch) = go_target(rust_target)?;
    ensure_native_go_target(os, arch, std::env::consts::OS, std::env::consts::ARCH)?;
    let producer_dir = producer_manifest
        .parent()
        .context("Go component manifest has no parent directory")?;
    let output = std::path::absolute(library)
        .with_context(|| format!("could not resolve Go component output {}", library.display()))?;
    let mut command = Command::new("go");
    command
        .current_dir(producer_dir)
        .env("CGO_ENABLED", "1")
        .env("GOOS", os)
        .env("GOARCH", arch)
        .args(["build", "-buildmode=c-shared", "-o"])
        .arg(output)
        .arg(".");
    Ok(command)
}

pub(super) fn go_target(rust_target: &str) -> Result<(&'static str, &'static str)> {
    match rust_target {
        "x86_64-unknown-linux-gnu" => Ok(("linux", "amd64")),
        "aarch64-unknown-linux-gnu" => Ok(("linux", "arm64")),
        "x86_64-apple-darwin" => Ok(("darwin", "amd64")),
        "aarch64-apple-darwin" => Ok(("darwin", "arm64")),
        "x86_64-pc-windows-gnu" | "x86_64-pc-windows-msvc" => Ok(("windows", "amd64")),
        "aarch64-pc-windows-gnullvm" | "aarch64-pc-windows-msvc" => Ok(("windows", "arm64")),
        _ => bail!("unsupported Go component target `{rust_target}`; use a native Linux GNU, macOS or Windows target"),
    }
}

pub(super) fn ensure_native_go_target(os: &str, arch: &str, host_os: &str, host_arch: &str) -> Result<()> {
    let host_os = if host_os == "macos" { "darwin" } else { host_os };
    let host_arch = match host_arch {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        other => other,
    };
    ensure!(
        (os, arch) == (host_os, host_arch),
        "Go component cross-compilation from {host_os}/{host_arch} to {os}/{arch} is unsupported; \
         build on the target platform with Go and a CGO-compatible C compiler installed"
    );
    Ok(())
}

fn execute_component_build(
    command: &mut Command,
    profile: &ComponentConfig,
    rust_target: &str,
    library: &Path,
) -> Result<()> {
    let tool = match profile.producer_language {
        ComponentProducerLanguage::Rust => "cargo",
        ComponentProducerLanguage::Go => "Go (CGO_ENABLED=1; requires a CGO-compatible C compiler)",
        ComponentProducerLanguage::Executable => "the executable build command",
    };
    if profile.producer_language == ComponentProducerLanguage::Go {
        let output_dir = library
            .parent()
            .context("Go component output has no parent directory")?;
        fs::create_dir_all(output_dir).with_context(|| {
            format!(
                "could not create Go component output directory {}",
                output_dir.display()
            )
        })?;
    }
    let status = command
        .status()
        .with_context(|| format!("failed to execute {tool} for component build"))?;
    ensure!(
        status.success(),
        "component `{}` build failed for target `{rust_target}` using {tool}",
        profile.name
    );
    Ok(())
}

/// Build the executable of a `producer_language = "executable"` component with its `build`
/// command, run through `sh -c` from the producer directory.
///
/// Cross-compilation is the command's business: alef only passes the target triple and the output
/// path, and checks that the file exists afterwards.
fn build_executable(
    config: &ResolvedCrateConfig,
    profile: &ComponentConfig,
    rust_target: &str,
    debug: bool,
    dry_run: bool,
    shared_target_dir: Option<&Path>,
) -> Result<()> {
    let executable = profile.executable.as_ref().with_context(|| {
        format!(
            "executable component `{}` has no `executable` configuration",
            profile.name
        )
    })?;
    let producer_dir = executable_producer_dir(config, profile);
    ensure!(
        producer_dir.is_dir(),
        "executable component producer directory does not exist: {}",
        producer_dir.display()
    );
    let output = std::path::absolute(built_library_path_for(
        config,
        profile,
        rust_target,
        debug,
        shared_target_dir,
    ))
    .context("could not resolve the executable output path")?;
    let out = output
        .to_str()
        .context("executable output path is not valid UTF-8")?
        .to_string();
    let script = executable.render_build(rust_target, &out, debug);
    tracing::info!(
        "Building executable component `{}` for `{rust_target}`: {script}",
        profile.name
    );
    if dry_run {
        return Ok(());
    }
    let output_dir = output.parent().context("executable output has no parent directory")?;
    fs::create_dir_all(output_dir)
        .with_context(|| format!("could not create executable output directory {}", output_dir.display()))?;
    if output.exists() {
        fs::remove_file(&output).with_context(|| format!("could not remove stale {}", output.display()))?;
    }
    let status = Command::new("sh")
        .arg("-c")
        .arg(&script)
        .current_dir(&producer_dir)
        .env("ALEF_COMPONENT_TARGET", rust_target)
        .env("ALEF_COMPONENT_OUT", &out)
        .env("ALEF_COMPONENT_PROFILE", if debug { "debug" } else { "release" })
        .status()
        .context("failed to execute `sh` for the executable component build")?;
    ensure!(
        status.success(),
        "component `{}` build command failed for target `{rust_target}`",
        profile.name
    );
    ensure!(
        output.is_file(),
        "component `{}` build command succeeded but did not produce {}",
        profile.name,
        output.display()
    );
    Ok(())
}

/// Directory of an executable component: its mandatory `producer_dir`.
pub(super) fn executable_producer_dir(config: &ResolvedCrateConfig, profile: &ComponentConfig) -> PathBuf {
    crate::core::config::component::component_producer_dir(
        &config.core_crate_dir(),
        &profile.name,
        profile.producer_dir.as_deref(),
    )
}

/// Where the built payload of `profile` for `target` lives, for every producer language.
pub(super) fn built_library_path_for(
    config: &ResolvedCrateConfig,
    profile: &ComponentConfig,
    target: &str,
    debug: bool,
    shared_target_dir: Option<&Path>,
) -> PathBuf {
    match &profile.executable {
        Some(executable) if profile.producer_language == ComponentProducerLanguage::Executable => {
            component_target_dir(config, &profile.name, shared_target_dir)
                .join(target)
                .join(if debug { "debug" } else { "release" })
                .join(executable.file_name(&profile.name, target))
        }
        _ => built_library_path(config, &profile.name, target, debug, shared_target_dir),
    }
}

/// Cargo target directory for one component build: the shared `--target-dir` when given, else a
/// private `.alef/components/build/<crate>/<component>` directory.
pub(super) fn component_target_dir(config: &ResolvedCrateConfig, component: &str, shared: Option<&Path>) -> PathBuf {
    if let Some(shared) = shared {
        return shared.to_path_buf();
    }
    Path::new(".alef")
        .join("components")
        .join("build")
        .join(&config.name)
        .join(component)
}

pub(super) fn built_library_path(
    config: &ResolvedCrateConfig,
    component: &str,
    target: &str,
    debug: bool,
    shared_target_dir: Option<&Path>,
) -> PathBuf {
    component_target_dir(config, component, shared_target_dir)
        .join(target)
        .join(if debug { "debug" } else { "release" })
        .join(dynamic_library_name(&producer_package_name(config, component), target))
}

fn producer_package_name(config: &ResolvedCrateConfig, component: &str) -> String {
    crate::core::config::component::component_producer_package(&config.core_crate_dir(), component)
}

/// `Cargo.toml` of the component's producer crate, honouring a configured `producer_dir`.
pub(super) fn producer_manifest_path(config: &ResolvedCrateConfig, profile: &ComponentConfig) -> PathBuf {
    crate::core::config::component::component_producer_dir(
        &config.core_crate_dir(),
        &profile.name,
        profile.producer_dir.as_deref(),
    )
    .join("Cargo.toml")
}
