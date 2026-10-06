use crate::snippets::error::Result;
use crate::snippets::scratch::ScratchDir;
use crate::snippets::session::ValidationSession;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::snippets::validators::{BatchValidation, SnippetValidator, run_command};

pub struct ZigValidator;

mod batch;
#[cfg(test)]
mod cache_dirs_tests;
pub(super) mod manifest;
#[cfg(test)]
mod session_command_tests;

/// Whether `zig` runs, not merely resolves: a version-manager shim spawns fine then exits
/// non-zero, so a PATH-only check leaves the skip below unreachable and fires the assert
/// everywhere Zig is absent. Shared by this file's own `mod tests`, `batch`'s tests, and
/// `session_command_tests`, which all gate real-`zig` tests the same way. ~keep
#[cfg(test)]
fn zig_is_runnable() -> bool {
    static RUNNABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *RUNNABLE.get_or_init(|| {
        std::process::Command::new("zig")
            .arg("version")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    })
}

impl SnippetValidator for ZigValidator {
    fn language(&self) -> Language {
        Language::Zig
    }

    fn is_available(&self) -> bool {
        which::which("zig").is_ok()
    }

    fn validate(
        &self,
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
    ) -> Result<(SnippetStatus, Option<String>)> {
        let dir = ScratchDir::isolated()?;
        let file = dir.path().join("snippet.zig");
        std::fs::write(&file, snippet.code.trim())?;

        let mut command = std::process::Command::new("zig");
        match level {
            ValidationLevel::Syntax => {
                command.arg("ast-check").arg(&file);
            }
            ValidationLevel::Compile | ValidationLevel::TypeCheck | ValidationLevel::Run => {
                command.args(["build-exe", "-fno-emit-bin"]).arg(&file);
            }
        }
        apply_cache_dirs(&mut command, dir.path(), None);

        let (success, output) = run_command(&mut command, timeout_secs)?;
        if success {
            Ok((SnippetStatus::Pass, None))
        } else {
            Ok((SnippetStatus::Fail, Some(output)))
        }
    }

    fn max_level(&self) -> ValidationLevel {
        ValidationLevel::Compile
    }

    fn missing_session_artifacts(
        &self,
        session: &ValidationSession,
        _level: ValidationLevel,
    ) -> Vec<std::path::PathBuf> {
        crate::snippets::validators::session_artifacts::missing_zig_ffi_library(session)
    }

    fn validate_in_session(
        &self,
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<(SnippetStatus, Option<String>)> {
        let Some(session) = session else {
            return self.validate(snippet, level, timeout_secs);
        };
        let dir = session.scratch_dir()?;
        let file = dir.path().join("snippet.zig");
        std::fs::write(&file, snippet.code.trim())?;
        let mut command = std::process::Command::new("zig");
        if level == ValidationLevel::Syntax {
            command.arg("ast-check");
        } else {
            command.args(["build-exe", "-fno-emit-bin"]);
        }
        let mut declared_include_paths = Vec::new();
        let mut uses_build_system = false;
        if level == ValidationLevel::Syntax {
            command.arg(&file);
        } else if let Some(manifest) = session.manifest.as_deref() {
            let (module_name, module_source) = zig_package_module(manifest)?;
            if let Some(package_root) = zig_package_root(&module_source) {
                // `alef build` with no `--release` produces `target/debug/`, but the scaffolded
                // `build.zig`'s `ffi_path` default only ever searches `target/release/` -- without
                // this override every snippet fails identically ("unable to find dynamic system
                // library") whenever the FFI crate was last built without `--release`. ~keep
                let ffi_override = manifest::resolve_ffi_library_override(manifest)?;
                let build_file = write_snippet_build(dir.path(), &module_name, &package_root, ffi_override.as_ref())?;
                command = std::process::Command::new("zig");
                command.args(["build", "--summary", "none", "--build-file"]);
                command.arg(build_file);
                // `-I` is a `zig build-exe` flag; `zig build` rejects it outright with
                // `unrecognized argument: '-I'` and fails the snippet before it compiles a line.
                // This path does not need one: the snippet's only import is the binding module,
                // and the package's own `build.zig` already declares its include directories, so
                // they reach the compilation through the dependency rather than the command line.
                // ~keep
                uses_build_system = true;
            } else {
                command
                    .args(["--dep", &module_name])
                    .arg(format!("-Mroot={}", file.display()))
                    .arg(format!("-M{module_name}={}", module_source.display()));
                // Resolved against the manifest's own directory, not the session's working
                // directory: the scaffolded `build.zig` rebases these defaults onto its build
                // root, and the two directories only coincide when the session happens to set
                // `cwd` to the package. ~keep
                let build_root = manifest.parent().unwrap_or_else(|| std::path::Path::new("."));
                declared_include_paths = manifest::zig_manifest_include_paths(manifest)?
                    .into_iter()
                    .map(|path| build_root.join(path))
                    .collect();
            }
        } else {
            command.arg(&file);
        }
        if !uses_build_system {
            apply_include_paths(&mut command, &session.include_paths);
            apply_include_paths(&mut command, &declared_include_paths);
        }
        apply_cache_dirs(&mut command, dir.path(), Some(session));
        session.apply(&mut command);
        let (success, output) = run_command(&mut command, timeout_secs)?;
        Ok(if success {
            (SnippetStatus::Pass, None)
        } else {
            (SnippetStatus::Fail, Some(output))
        })
    }

    /// Only the AST level batches. `Compile` builds one executable from one root file — and zig
    /// analyses a declaration only where it is referenced, so aggregating N snippets behind
    /// `@import` would leave most of their code unanalysed and passing on that basis — so it falls
    /// back to one process per snippet. ~keep
    fn validate_batch_in_session(
        &self,
        snippets: &[&Snippet],
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Option<Result<BatchValidation>> {
        (level == ValidationLevel::Syntax).then(|| batch::validate_batch_with_context(snippets, timeout_secs, session))
    }

    fn supports_batching(&self) -> bool {
        true
    }

    fn is_dependency_error(&self, output: &str) -> bool {
        output.contains("unable to find") || output.contains("@import")
    }
}

/// Points zig's caches at a directory this invocation controls, rather than letting zig resolve
/// one from `HOME`/`XDG_CACHE_HOME` -- `run_command`'s `sanitize_environment` allowlist carries
/// neither, so without an explicit override zig aborts with `error: unable to resolve zig cache
/// directory: AppDataDirUnavailable` before it reads a single line of the snippet, and every zig
/// snippet fails identically at compile level in a way that looks like a defect in the snippet.
///
/// `ZIG_LOCAL_CACHE_DIR` is always scoped to `dir`, the caller's own scratch directory for this
/// invocation -- zig's local cache holds incremental state for one specific build and is not
/// designed to be shared across snippets with different source.
///
/// `ZIG_GLOBAL_CACHE_DIR` is different: when `session` is `Some`, this function deliberately does
/// *not* set it, because `ValidationSession::apply`/`apply_environment` (called by every caller
/// immediately after this one) sets it to a fingerprint-scoped directory that persists across runs
/// and is shared by every zig snippet validated under that session -- zig's global cache is
/// content-addressed and explicitly designed for concurrent sharing across processes (this is how
/// `zig build` itself parallelizes sub-compilations against one cache), so sharing it here is safe,
/// not merely convenient. Setting it here too used to work only by accident of call order --
/// `Command::env` is last-write-wins, and every caller happened to call `session.apply` second, so
/// this function's own `ZIG_GLOBAL_CACHE_DIR` write was silently shadowed. That made the sharing
/// depend on an ordering invariant nothing enforced: reordering the two calls, or introducing a
/// caller that didn't, would silently regress every zig snippet in that session back to a fresh,
/// unshared, `--clean`-cold global cache with no test failure to catch it. Making the two paths
/// mutually exclusive here removes the ordering dependency entirely: with a session, only
/// `session.apply` ever sets `ZIG_GLOBAL_CACHE_DIR`, full stop. Without one (the standalone,
/// no-session `validate` path) there is no session cache to share against, so it falls back to the
/// same scratch directory as the local cache, exactly as before. ~keep
fn apply_cache_dirs(command: &mut std::process::Command, dir: &std::path::Path, session: Option<&ValidationSession>) {
    command.env("ZIG_LOCAL_CACHE_DIR", dir.join("zig-local-cache"));
    if session.is_none() {
        command.env("ZIG_GLOBAL_CACHE_DIR", dir.join("zig-global-cache"));
    }
}

fn apply_include_paths(command: &mut std::process::Command, include_paths: &[std::path::PathBuf]) {
    for include_path in include_paths {
        command.arg("-I").arg(include_path);
    }
}

fn zig_package_module(manifest: &std::path::Path) -> Result<(String, std::path::PathBuf)> {
    let source = std::fs::read_to_string(manifest)?;
    let module_marker = "addModule(\"";
    let module_start = source.find(module_marker).ok_or_else(|| {
        crate::snippets::error::Error::Other(format!("no addModule declaration in {}", manifest.display()))
    })? + module_marker.len();
    let module_end = source[module_start..].find('"').ok_or_else(|| {
        crate::snippets::error::Error::Other(format!("invalid addModule declaration in {}", manifest.display()))
    })? + module_start;
    let root_marker = "root_source_file = b.path(\"";
    let root_start = source[module_end..].find(root_marker).ok_or_else(|| {
        crate::snippets::error::Error::Other(format!("no module root source in {}", manifest.display()))
    })? + module_end
        + root_marker.len();
    let root_end = source[root_start..].find('"').ok_or_else(|| {
        crate::snippets::error::Error::Other(format!("invalid module root source in {}", manifest.display()))
    })? + root_start;
    let root = manifest
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join(&source[root_start..root_end]);
    Ok((source[module_start..module_end].to_owned(), root))
}

fn zig_package_root(module_source: &std::path::Path) -> Option<std::path::PathBuf> {
    module_source.ancestors().find_map(|ancestor| {
        (ancestor.join("build.zig").is_file() && ancestor.join("build.zig.zon").is_file())
            .then(|| ancestor.to_path_buf())
    })
}

fn write_snippet_build(
    directory: &std::path::Path,
    module_name: &str,
    package_root: &std::path::Path,
    ffi_override: Option<&(String, std::path::PathBuf)>,
) -> Result<std::path::PathBuf> {
    // Zig 0.16 requires `.path` dependencies in `build.zig.zon` to be relative to the build
    // root (the manifest's own directory, i.e. `directory` here) — an absolute path is a hard
    // `zig build` error (`expected path relative to build root; found absolute path`), not a
    // lint warning. `package_root` arrives absolute (from `zig_package_root`, which walks up
    // from an absolute manifest path), so it must be rebased here rather than written as-is. ~keep
    let package_root = zon_dependency_path(&relative_path(directory, package_root)?);
    // A top-level `-D` flag only sets an option on *this* build.zig, never on a `.path`
    // dependency's own `b.option(...)` calls -- those are set only by naming them in the
    // `b.dependency(...)` args struct itself, which is why the override is spliced into the
    // dependency call here rather than passed on the command line. ~keep
    let dependency_args = match ffi_override {
        Some((option_name, path)) => format!(
            ".target = target, .optimize = optimize, .{option_name} = \"{}\"",
            zig_string_literal(path)
        ),
        None => ".target = target, .optimize = optimize".to_owned(),
    };
    let build = format!(
        "const std = @import(\"std\");\n\npub fn build(b: *std.Build) void {{\n    const target = b.standardTargetOptions(.{{}});\n    const optimize = b.standardOptimizeOption(.{{}});\n    const binding = b.dependency(\"binding\", .{{ {dependency_args} }});\n    const root = b.createModule(.{{\n        .root_source_file = b.path(\"snippet.zig\"),\n        .target = target,\n        .optimize = optimize,\n    }});\n    root.addImport(\"{module_name}\", binding.module(\"{module_name}\"));\n    const executable = b.addExecutable(.{{ .name = \"snippet\", .root_module = root }});\n    b.default_step.dependOn(&executable.step);\n}}\n"
    );
    let zon = format!(
        ".{{\n    .name = .alef_snippet,\n    .version = \"0.0.0\",\n    .fingerprint = 0x{fingerprint:016x},\n    .dependencies = .{{ .binding = .{{ .path = \"{package_root}\" }} }},\n    .paths = .{{ \"build.zig\", \"build.zig.zon\", \"snippet.zig\" }},\n}}\n",
        fingerprint = snippet_package_fingerprint(),
    );
    let build_file = directory.join("build.zig");
    std::fs::write(&build_file, build)?;
    std::fs::write(directory.join("build.zig.zon"), zon)?;
    Ok(build_file)
}

/// Render `relative` as the string a `build.zig.zon` `.path` field must hold.
///
/// Joins the path's *components* with `/` rather than formatting the path itself. On Windows
/// `Path` renders with `\`, and Zig resolves `.path` dependencies POSIX-style, so a native
/// rendering reached the manifest as a single nonsensical component (`..\\package`) and the
/// dependency could not be fetched. Forward slashes are what Zig accepts on every platform. ~keep
fn zon_dependency_path(relative: &std::path::Path) -> String {
    relative
        .components()
        .map(|component| {
            component
                .as_os_str()
                .to_string_lossy()
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Render `path` as a Zig string literal body (escaped, unquoted). Unlike
/// [`zon_dependency_path`], the result is used as an ordinary `.cwd_relative` filesystem path
/// rather than a `build.zig.zon` `.path` dependency, so native separators are left as-is -- only
/// the characters a Zig string literal cannot contain unescaped are escaped. ~keep
fn zig_string_literal(path: &std::path::Path) -> String {
    path.display().to_string().replace('\\', "\\\\").replace('"', "\\\"")
}

/// Express `target` as a path relative to `base`, purely lexically (no filesystem access, so
/// it works even when `target` does not exist yet — unlike `Path::canonicalize`-based
/// approaches, which also risk silently resolving macOS's `/tmp` → `/private/tmp` symlink and
/// producing a technically-different-but-equivalent root).
///
/// Errors instead of falling back to an absolute path when no relative path can be expressed —
/// e.g. `base` and `target` disagree on being absolute, or (Windows) sit on different drive
/// prefixes. Emitting an absolute path anyway would just move the failure from loud, at
/// generation time, to a `zig build` error a consumer has to reverse-engineer — the same
/// silence-vs-loudness principle the rest of this fix applies. ~keep
fn relative_path(base: &std::path::Path, target: &std::path::Path) -> Result<std::path::PathBuf> {
    use std::path::Component;

    if base.is_absolute() != target.is_absolute() {
        return Err(crate::snippets::error::Error::Other(format!(
            "cannot express {} relative to {}: one is absolute and the other is not",
            target.display(),
            base.display()
        )));
    }

    let base_components: Vec<Component> = base.components().collect();
    let target_components: Vec<Component> = target.components().collect();
    let first_pair = (base_components.first(), target_components.first());
    if let (Some(Component::Prefix(a)), Some(Component::Prefix(b))) = first_pair
        && a.as_os_str() != b.as_os_str()
    {
        return Err(crate::snippets::error::Error::Other(format!(
            "cannot express {} relative to {}: no common root",
            target.display(),
            base.display()
        )));
    }

    let common = base_components
        .iter()
        .zip(target_components.iter())
        .take_while(|(a, b)| a == b)
        .count();

    let mut relative = std::path::PathBuf::new();
    for _ in common..base_components.len() {
        relative.push("..");
    }
    for component in &target_components[common..] {
        relative.push(component.as_os_str());
    }

    Ok(if relative.as_os_str().is_empty() {
        std::path::PathBuf::from(".")
    } else {
        relative
    })
}

/// Deterministic fingerprint for the synthetic `.alef_snippet` scratch package every
/// session-scoped Zig snippet build writes to a temp dir. Zig 0.16 rejects a `build.zig.zon`
/// with no top-level `.fingerprint` (`(crc32_ieee(name) << 32) | id`, `id` never `0`/`0xffff_ffff`)
/// — without one, every session-scoped snippet failed during manifest parsing, before any
/// snippet code was read. `alef.toml`'s `minimum_zig_version` floor is 0.17.0
/// (`toolchain::MIN_ZIG_VERSION`), so this is unconditional, matching
/// `scaffold::languages::zig::zig_fingerprint`'s same choice for real scaffolded packages.
///
/// Duplicated rather than shared across the `scaffold`/`snippets` module boundary: this
/// scratch package's identity is unrelated to any scaffolded crate's and is derived from the
/// fixed name below, so the value is always the same — intentional, since this package is
/// never published or fetched, only compiled locally for validation. ~keep
fn snippet_package_fingerprint() -> u64 {
    const NAME: &[u8] = b"alef_snippet";
    let name_crc = crc32_ieee(NAME);
    let mut id: u32 = 0x811c_9dc5;
    for byte in NAME {
        id ^= *byte as u32;
        id = id.wrapping_mul(0x0100_0193);
    }
    if id == 0 || id == 0xffff_ffff {
        id = 0x1;
    }
    ((name_crc as u64) << 32) | (id as u64)
}

/// IEEE CRC-32, the half of the Zig fingerprint scheme `crc32_ieee(name)` needs.
fn crc32_ieee(bytes: &[u8]) -> u32 {
    let mut crc: u32 = 0xffff_ffff;
    for byte in bytes {
        crc ^= *byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests;
