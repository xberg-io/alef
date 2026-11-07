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
        ComponentProducerLanguage::Executable => {
            unreachable!(
                "executable components are built by `build_executable`, which `build_component` dispatches to first"
            )
        }
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
    crate::component::fingerprint::ensure_shell(std::env::var_os("PATH").as_deref())?;
    ensure_outside_producer(&producer_dir, &output)?;
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
    if !status.success() {
        // A half-written payload must not survive to be packaged or mistaken for a build.
        let _ = fs::remove_file(&output);
        bail!(
            "component `{}` build command failed for target `{rust_target}` ({status})",
            profile.name
        );
    }
    ensure!(
        output.is_file(),
        "component `{}` build command succeeded but did not produce {}",
        profile.name,
        output.display()
    );
    make_executable(&output)
}

/// A payload that is not executable cannot be served; the build command is not trusted to set it.
#[cfg(unix)]
fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
        .with_context(|| format!("could not mark {} executable", path.display()))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<()> {
    Ok(())
}

/// The build output of an executable component must not sit inside its own producer directory:
/// that directory is the default fingerprint input, so the payload would change its own hash.
fn ensure_outside_producer(producer_dir: &Path, output: &Path) -> Result<()> {
    let clean = |path: &Path| {
        crate::core::config::component::lexically_clean(
            &std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()),
        )
    };
    ensure!(
        !clean(output).starts_with(clean(producer_dir)),
        "executable component output {} lies inside its producer_dir {}; choose a producer_dir that does not contain \
         the alef build directory or the --target-dir",
        output.display(),
        producer_dir.display()
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
            built_dir(config, &profile.name, target, debug, shared_target_dir)
                .join(executable.file_name(&profile.name, target))
        }
        _ => built_library_path(config, &profile.name, target, debug, shared_target_dir),
    }
}

/// alef's own working directory, relative to the working directory; never a fingerprint input.
pub(super) fn alef_build_root() -> PathBuf {
    PathBuf::from(".alef")
}

/// Cargo target directory for one component build: the shared `--target-dir` when given, else a
/// private `.alef/components/build/<crate>/<component>` directory.
pub(super) fn component_target_dir(config: &ResolvedCrateConfig, component: &str, shared: Option<&Path>) -> PathBuf {
    if let Some(shared) = shared {
        return shared.to_path_buf();
    }
    alef_build_root()
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
    built_dir(config, component, target, debug, shared_target_dir)
        .join(dynamic_library_name(&producer_package_name(config, component), target))
}

/// `<target dir>/<target>/<profile>`: where a component build leaves its payload.
fn built_dir(
    config: &ResolvedCrateConfig,
    component: &str,
    target: &str,
    debug: bool,
    shared_target_dir: Option<&Path>,
) -> PathBuf {
    component_target_dir(config, component, shared_target_dir)
        .join(target)
        .join(if debug { "debug" } else { "release" })
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
