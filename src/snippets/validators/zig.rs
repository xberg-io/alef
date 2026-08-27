use crate::snippets::error::Result;
use crate::snippets::scratch::ScratchDir;
use crate::snippets::session::ValidationSession;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::snippets::validators::{BatchValidation, SnippetValidator, run_command};

pub struct ZigValidator;

mod batch;
mod manifest;
#[cfg(test)]
mod session_command_tests;

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
        apply_cache_dirs(&mut command, dir.path());

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
        apply_cache_dirs(&mut command, dir.path());
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

/// Point zig's caches inside the snippet's own temp directory.
///
/// ~keep zig resolves its cache directory from `HOME`/`XDG_CACHE_HOME`, and `run_command`'s
/// `sanitize_environment` allowlist carries neither. Without these variables zig aborts with
/// `error: unable to resolve zig cache directory: AppDataDirUnavailable` before it reads a single
/// line of the snippet, so every zig snippet fails identically at compile level and the failure
/// looks like a defect in the snippet. Setting them explicitly keeps the run hermetic instead of
/// widening the allowlist, which would leak the developer's real zig cache into validation.
fn apply_cache_dirs(command: &mut std::process::Command, dir: &std::path::Path) {
    command.env("ZIG_GLOBAL_CACHE_DIR", dir.join("zig-global-cache"));
    command.env("ZIG_LOCAL_CACHE_DIR", dir.join("zig-local-cache"));
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
/// snippet code was read. `alef.toml`'s `minimum_zig_version` floor is 0.16.0
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
mod tests {
    use super::*;
    use crate::snippets::types::{SnippetMetadata, SnippetStatus, SourceOrigin};
    use std::path::PathBuf;

    const TOOLCHAIN_TEST_TIMEOUT_SECS: u64 = 120;

    #[test]
    fn compiles_a_snippet_under_the_sanitized_environment() {
        if which::which("zig").is_err() {
            return;
        }
        let snippet =
            zig_snippet("const std = @import(\"std\");\n\npub fn main() void {\n    _ = std.mem.zeroes(u8);\n}\n");

        let (status, output) = ZigValidator
            .validate(&snippet, ValidationLevel::Compile, TOOLCHAIN_TEST_TIMEOUT_SECS)
            .expect("validation runs");

        assert_eq!(
            status,
            SnippetStatus::Pass,
            "zig must compile under the sanitized environment; without an explicit cache directory it \
             fails with AppDataDirUnavailable before reading the snippet: {output:?}"
        );
    }

    #[test]
    fn cache_directories_are_scoped_to_the_snippet_directory() {
        let root = tempfile::tempdir().expect("temporary root");
        let mut command = std::process::Command::new("zig");
        apply_cache_dirs(&mut command, root.path());

        let configured: Vec<_> = command
            .get_envs()
            .filter_map(|(key, value)| value.map(|value| (key.to_string_lossy().into_owned(), PathBuf::from(value))))
            .collect();

        assert_eq!(
            configured,
            vec![
                ("ZIG_GLOBAL_CACHE_DIR".to_string(), root.path().join("zig-global-cache")),
                ("ZIG_LOCAL_CACHE_DIR".to_string(), root.path().join("zig-local-cache")),
            ]
        );
    }

    fn zig_snippet(code: &str) -> Snippet {
        Snippet {
            id: None,
            path: PathBuf::from("snippet.zig"),
            language: Language::Zig,
            title: None,
            code: code.into(),
            start_line: 1,
            block_index: 0,
            annotation: None,
            metadata: SnippetMetadata::default(),
            source_origin: SourceOrigin {
                path: PathBuf::from("snippet.zig"),
                line: 1,
                block_index: 0,
            },
        }
    }

    #[test]
    fn resolves_declared_package_module() {
        let directory = tempfile::tempdir().unwrap();
        let manifest = directory.path().join("build.zig");
        std::fs::write(
            &manifest,
            "const module = b.addModule(\"sample_binding\", .{\n    .root_source_file = b.path(\"src/root.zig\"),\n});\n",
        )
        .unwrap();
        let (name, source) = zig_package_module(&manifest).unwrap();
        assert_eq!(name, "sample_binding");
        assert_eq!(source, directory.path().join("src/root.zig"));
    }

    #[test]
    fn snippet_build_reuses_the_generated_package_dependency_graph() {
        let directory = tempfile::tempdir().unwrap();
        let package = directory.path().join("package");
        std::fs::create_dir(&package).unwrap();
        std::fs::write(package.join("build.zig"), "").unwrap();
        std::fs::write(package.join("build.zig.zon"), "").unwrap();
        let scratch = directory.path().join("scratch");
        std::fs::create_dir(&scratch).unwrap();

        let build_file = write_snippet_build(&scratch, "sample_binding", &package, None).unwrap();
        let build = std::fs::read_to_string(build_file).unwrap();
        let zon = std::fs::read_to_string(scratch.join("build.zig.zon")).unwrap();

        assert!(build.contains("binding.module(\"sample_binding\")"), "{build}");
        assert!(build.contains("root.addImport(\"sample_binding\""), "{build}");
        // `scratch` and `package` are siblings under `directory`, so `package` relative to
        // `scratch` is `../package` — not the absolute `package` path. Zig 0.16 rejects an
        // absolute `.path` dependency outright (`expected path relative to build root; found
        // absolute path`), so this pins the regression, not just the presence of a `.path` key.
        assert!(zon.contains(".path = \"../package\""), "{zon}");
        assert!(
            !zon.contains(&format!(".path = \"{}\"", package.display())),
            "dependency .path must be relative to the manifest's own directory, not absolute; got:\n{zon}"
        );
        assert!(
            zon.contains(".fingerprint = 0x"),
            "Zig 0.16 rejects a build.zig.zon with no top-level .fingerprint field; got:\n{zon}"
        );
    }

    /// `build.zig.zon` `.path` values are resolved POSIX-style by Zig on every platform, so the
    /// rendering must not follow the host's separator. Driving this from `Path` *components* is
    /// the point: `Path::new("..").join("package")` carries exactly the component sequence
    /// Windows renders as `..\package`, so this exercises the Windows input shape on any host --
    /// the previous `to_string_lossy()` rendering turned that same sequence into a single
    /// unfetchable component. ~keep
    #[test]
    fn a_zon_dependency_path_is_rendered_with_forward_slashes() {
        assert_eq!(zon_dependency_path(&PathBuf::from("..").join("package")), "../package");
        assert_eq!(
            zon_dependency_path(&PathBuf::from("..").join("..").join("a").join("b")),
            "../../a/b"
        );
        assert_eq!(zon_dependency_path(std::path::Path::new(".")), ".");
    }

    /// The emitted manifest must carry no separator Zig would refuse, whatever the host. ~keep
    #[test]
    fn a_zon_manifest_never_carries_a_host_path_separator() {
        let directory = tempfile::tempdir().unwrap();
        let package = directory.path().join("package");
        std::fs::create_dir(&package).unwrap();
        let scratch = directory.path().join("scratch");
        std::fs::create_dir(&scratch).unwrap();

        write_snippet_build(&scratch, "sample_binding", &package, None).unwrap();
        let zon = std::fs::read_to_string(scratch.join("build.zig.zon")).unwrap();

        let dependency_line = zon
            .lines()
            .find(|line| line.contains(".path = "))
            .expect("the manifest declares a path dependency");
        assert!(
            !dependency_line.contains('\\'),
            "a backslash in a .zon dependency path is not a separator to Zig; got: {dependency_line}"
        );
        assert!(dependency_line.contains(".path = \"../package\""), "{dependency_line}");
    }

    /// `/tmp/...` is not an absolute path on Windows -- it has no drive prefix, so
    /// `Path::is_absolute` is false and the absolute/relative mismatch these tests turn on never
    /// arises. Building the root per platform keeps them testing what they claim. ~keep
    fn absolute(tail: &str) -> PathBuf {
        let root = if cfg!(windows) { r"C:\" } else { "/" };
        PathBuf::from(root).join(tail)
    }

    /// Regression: Zig 0.16 rejects an absolute `.path` dependency in `build.zig.zon` outright
    /// (`expected path relative to build root; found absolute path`) — this is a real `zig
    /// build` error, not a style nit, and it fired on every session-scoped snippet whose package
    /// root was written verbatim (always absolute — `zig_package_root` walks up from an
    /// absolute manifest path). ~keep
    #[test]
    fn relative_path_rebases_a_deeper_absolute_target_onto_a_shallower_base() {
        let base = absolute("tmp/session/scratch");
        let target = absolute("tmp/session/package/nested");

        let relative = relative_path(&base, &target).unwrap();

        assert_eq!(relative, PathBuf::from("../package/nested"));
    }

    #[test]
    fn relative_path_is_dot_when_base_and_target_are_the_same_directory() {
        let dir = absolute("tmp/session/scratch");

        let relative = relative_path(&dir, &dir).unwrap();

        assert_eq!(relative, PathBuf::from("."));
    }

    /// A silently-wrong absolute path is exactly the failure mode being fixed — if no relative
    /// path can be expressed, generation must fail loudly instead of falling back to one.
    #[test]
    fn relative_path_errors_rather_than_falling_back_to_absolute() {
        let rooted = absolute("tmp/session/package");
        let relative = PathBuf::from("package");

        let err = relative_path(&rooted, &relative).unwrap_err();

        assert!(
            err.to_string().contains("absolute"),
            "error should explain the absolute/relative mismatch; got: {err}"
        );
    }

    /// Regression: without a `.fingerprint` field, Zig 0.16 fails every session-scoped snippet
    /// during manifest parsing, before any snippet code is read — this is the exact shape of
    /// bug the vacuous-target defect hunt was about, just one level down in the toolchain rather
    /// than in generated bindings. Pins the field's shape and the value's determinism, since a
    /// randomly-generated fingerprint would churn the manifest (irrelevant for this scratch file,
    /// which is never committed, but the algorithm is shared in spirit with
    /// `scaffold::languages::zig::zig_fingerprint`, which does need determinism for committed
    /// output — so this also guards against the two silently diverging). ~keep
    #[test]
    fn snippet_package_fingerprint_is_deterministic_and_zig_valid() {
        let first = snippet_package_fingerprint();
        let second = snippet_package_fingerprint();
        assert_eq!(first, second, "fingerprint must be stable across calls, not random");

        let id = (first & 0xffff_ffff) as u32;
        assert_ne!(id, 0, "Zig rejects id == 0");
        assert_ne!(id, 0xffff_ffff, "Zig rejects id == 0xffffffff");
    }

    /// The strongest available proof the manifest is well-formed: actually run `zig build`
    /// against it, rather than `zig ast-check` on a single unrelated file (which does not parse
    /// `build.zig.zon` at all and would pass green even with a missing `.fingerprint`). ~keep
    #[test]
    fn snippet_build_zon_parses_under_real_zig() {
        if which::which("zig").is_err() {
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let package = directory.path().join("package");
        std::fs::create_dir(&package).unwrap();
        let package_fingerprint = {
            const NAME: &[u8] = b"binding_pkg";
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
        };
        std::fs::write(
            package.join("build.zig"),
            "const std = @import(\"std\");\n\npub fn build(b: *std.Build) void {\n    \
             const target = b.standardTargetOptions(.{});\n    \
             const optimize = b.standardOptimizeOption(.{});\n    \
             _ = b.addModule(\"binding_pkg\", .{\n        \
                 .root_source_file = b.path(\"root.zig\"),\n        \
                 .target = target,\n        \
                 .optimize = optimize,\n    \
             });\n}\n",
        )
        .unwrap();
        std::fs::write(package.join("root.zig"), "pub const ok = true;\n").unwrap();
        std::fs::write(
            package.join("build.zig.zon"),
            format!(
                ".{{\n    .name = .binding_pkg,\n    .version = \"0.0.0\",\n    \
                 .fingerprint = 0x{package_fingerprint:016x},\n    \
                 .minimum_zig_version = \"0.16.0\",\n    \
                 .paths = .{{ \"build.zig\", \"build.zig.zon\", \"root.zig\" }},\n}}\n"
            ),
        )
        .unwrap();

        let scratch = directory.path().join("scratch");
        std::fs::create_dir(&scratch).unwrap();
        let build_file = write_snippet_build(&scratch, "binding_pkg", &package, None).unwrap();
        std::fs::write(
            scratch.join("snippet.zig"),
            "const binding_pkg = @import(\"binding_pkg\");\npub fn main() void {\n    _ = binding_pkg.ok;\n}\n",
        )
        .unwrap();

        let mut command = std::process::Command::new("zig");
        command
            .args(["build", "--summary", "none", "--build-file"])
            .arg(&build_file);
        apply_cache_dirs(&mut command, &scratch);
        let output = command.output().expect("zig must be installed to verify this test");

        assert!(
            output.status.success(),
            "zig build failed against the generated snippet manifest:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// The decisive check: a snippet whose binding module reaches a `@cInclude` compiles only when
    /// the include directory the manifest declares reaches the reconstructed `build-exe` command.
    #[test]
    fn a_snippet_compiles_against_the_include_path_its_manifest_declares() {
        if which::which("zig").is_err() {
            return;
        }

        let (declaring, declaring_session) = sample_project(true);
        let (omitting, omitting_session) = sample_project(false);
        let snippet = zig_snippet(
            "const sample_binding = @import(\"sample_binding\");\n\npub fn main() void {\n    _ = sample_binding.value();\n}\n",
        );

        let (declared_status, declared_output) = ZigValidator
            .validate_in_session(
                &snippet,
                ValidationLevel::Compile,
                TOOLCHAIN_TEST_TIMEOUT_SECS,
                Some(&declaring_session),
            )
            .expect("declaring session validates");
        let (omitted_status, _) = ZigValidator
            .validate_in_session(
                &snippet,
                ValidationLevel::Compile,
                TOOLCHAIN_TEST_TIMEOUT_SECS,
                Some(&omitting_session),
            )
            .expect("omitting session validates");

        assert_eq!(
            declared_status,
            SnippetStatus::Pass,
            "the manifest declares the include directory, so the header must resolve: {declared_output:?}"
        );
        assert_eq!(
            omitted_status,
            SnippetStatus::Fail,
            "without a declared include directory the header cannot resolve"
        );
        drop((declaring, omitting));
    }

    /// The decisive cwd-independence check, and the exact shape the Zig snippet validator itself
    /// builds: the scaffolded package is consumed as a `.path` dependency from a scratch directory
    /// while zig runs with its working directory somewhere else entirely. A `.cwd_relative` search
    /// path built from a raw relative default cannot resolve from there — which is why the paired
    /// negative control below must fail. ~keep
    #[test]
    fn a_snippet_compiles_when_the_package_is_built_from_an_unrelated_working_directory() {
        if which::which("zig").is_err() {
            return;
        }

        let (rebased, rebased_session) = sample_package(true);
        let (raw, raw_session) = sample_package(false);
        let snippet = zig_snippet(
            "const sample_package = @import(\"sample_package\");\n\npub fn main() void {\n    _ = sample_package.value();\n}\n",
        );

        let (rebased_status, rebased_output) = ZigValidator
            .validate_in_session(
                &snippet,
                ValidationLevel::Compile,
                TOOLCHAIN_TEST_TIMEOUT_SECS,
                Some(&rebased_session),
            )
            .expect("rebased session validates");
        let (raw_status, _) = ZigValidator
            .validate_in_session(
                &snippet,
                ValidationLevel::Compile,
                TOOLCHAIN_TEST_TIMEOUT_SECS,
                Some(&raw_session),
            )
            .expect("raw session validates");

        assert_eq!(
            rebased_status,
            SnippetStatus::Pass,
            "rebasing onto the build root makes the include directory resolve from any cwd: {rebased_output:?}"
        );
        assert_eq!(
            raw_status,
            SnippetStatus::Fail,
            "a raw cwd-relative default cannot resolve from a foreign working directory"
        );
        drop((rebased, raw));
    }

    /// A complete Zig package (`build.zig` + `build.zig.zon`) whose module only compiles when its
    /// include directory resolves, paired with a session whose working directory is a *sibling* of
    /// the package rather than the package itself. `rebase_onto_build_root` selects between the
    /// declaration alef emits today and the raw `.cwd_relative` one it emitted before.
    fn sample_package(rebase_onto_build_root: bool) -> (tempfile::TempDir, ValidationSession) {
        const PACKAGE: &str = "sample_package";

        let directory = tempfile::tempdir().expect("project directory");
        let package = directory.path().join("package");
        let elsewhere = directory.path().join("elsewhere");
        std::fs::create_dir_all(package.join("src")).unwrap();
        std::fs::create_dir_all(package.join("vendor/include")).unwrap();
        std::fs::create_dir_all(&elsewhere).unwrap();

        let manifest = if rebase_onto_build_root {
            manifest::tests::build_root_rebased_build_zig(PACKAGE)
        } else {
            manifest::tests::build_root_rebased_build_zig(PACKAGE).replace(
                "const ffi_include = b.pathResolve(&.{ build_root, ffi_include_option });",
                "const ffi_include = ffi_include_option;",
            )
        };
        std::fs::write(package.join("build.zig"), manifest).unwrap();
        std::fs::write(
            package.join("build.zig.zon"),
            format!(
                ".{{\n    .name = .{PACKAGE},\n    .version = \"0.0.0\",\n    \
                 .fingerprint = 0x{fingerprint:016x},\n    \
                 .minimum_zig_version = \"0.16.0\",\n    \
                 .paths = .{{ \"build.zig\", \"build.zig.zon\", \"src\", \"vendor\" }},\n}}\n",
                fingerprint = package_fingerprint(PACKAGE.as_bytes()),
            ),
        )
        .unwrap();
        std::fs::write(package.join("vendor/include/fixture.h"), "#define FIXTURE_VALUE 7\n").unwrap();
        std::fs::write(
            package.join("src/root.zig"),
            "pub const c = @cImport(@cInclude(\"fixture.h\"));\n\npub fn value() c_int {\n    return c.FIXTURE_VALUE;\n}\n",
        )
        .unwrap();

        let session = ValidationSession {
            language: Language::Zig,
            working_directory: elsewhere,
            manifest: Some(package.join("build.zig")),
            fingerprint: "foreign-cwd-project".into(),
            env: std::collections::BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: std::collections::BTreeMap::new(),
        };
        (directory, session)
    }

    /// Zig 0.16's `build.zig.zon` fingerprint scheme, for fixture packages `zig build` must accept.
    fn package_fingerprint(name: &[u8]) -> u64 {
        let mut id: u32 = 0x811c_9dc5;
        for byte in name {
            id ^= u32::from(*byte);
            id = id.wrapping_mul(0x0100_0193);
        }
        if id == 0 || id == 0xffff_ffff {
            id = 0x1;
        }
        (u64::from(crc32_ieee(name)) << 32) | u64::from(id)
    }

    /// A self-contained Zig project whose module only compiles when its declared include directory
    /// is on the search path. Returns the temp dir so the caller keeps it alive.
    fn sample_project(with_include: bool) -> (tempfile::TempDir, ValidationSession) {
        let directory = tempfile::tempdir().expect("project directory");
        let root = directory.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("vendor/include")).unwrap();
        std::fs::write(root.join("build.zig"), manifest::tests::sample_build_zig(with_include)).unwrap();
        std::fs::write(root.join("vendor/include/fixture.h"), "#define FIXTURE_VALUE 7\n").unwrap();
        std::fs::write(
            root.join("src/root.zig"),
            "pub const c = @cImport(@cInclude(\"fixture.h\"));\n\npub fn value() c_int {\n    return c.FIXTURE_VALUE;\n}\n",
        )
        .unwrap();

        let session = ValidationSession {
            language: Language::Zig,
            working_directory: root.to_path_buf(),
            manifest: Some(root.join("build.zig")),
            fingerprint: "neutral-project".into(),
            env: std::collections::BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: std::collections::BTreeMap::new(),
        };
        (directory, session)
    }

    /// A `.path`-dependency package whose `ffi_path` default names a `target/release/` directory
    /// that is never created; only `target/debug/` carries a real, linkable dynamic library. Mirrors
    /// exactly what `alef build` with no `--release` flag leaves on disk.
    fn debug_only_ffi_project() -> (tempfile::TempDir, ValidationSession) {
        const LIB_NAME: &str = "sample_ffi";

        let directory = tempfile::tempdir().expect("project directory");
        let debug_dir = directory.path().join("target/debug");
        std::fs::create_dir_all(&debug_dir).unwrap();
        let lib_source = directory.path().join("fixture.zig");
        std::fs::write(
            &lib_source,
            "export fn fixture_value() callconv(.c) c_int {\n    return 7;\n}\n",
        )
        .unwrap();
        // The name the host toolchain really produces, which on Windows is `{name}.dll` with no
        // `lib` prefix. Writing `lib{name}.dll` there made this fixture agree with a probe that
        // was itself wrong, and pinned a filename the `zig build` below could never have linked:
        // zig searches `{name}.dll`, `{name}.lib`, `lib{name}.a` on Windows and nothing else. ~keep
        let lib_path = debug_dir.join(if cfg!(target_os = "windows") {
            format!("{LIB_NAME}.dll")
        } else if cfg!(target_os = "macos") {
            format!("lib{LIB_NAME}.dylib")
        } else {
            format!("lib{LIB_NAME}.so")
        });
        let mut lib_command = std::process::Command::new("zig");
        lib_command
            .args(["build-lib", "-dynamic"])
            .arg(&lib_source)
            .arg(format!("-femit-bin={}", lib_path.display()));
        apply_cache_dirs(&mut lib_command, directory.path());
        let lib_output = lib_command
            .output()
            .expect("zig must be installed to build the fixture library");
        assert!(
            lib_output.status.success(),
            "failed to build the fixture FFI library:\n{}",
            String::from_utf8_lossy(&lib_output.stderr)
        );

        let package = directory.path().join("package");
        std::fs::create_dir_all(package.join("src")).unwrap();
        std::fs::write(
            package.join("build.zig"),
            manifest::tests::build_root_rebased_ffi_path_build_zig(LIB_NAME, "../target/release"),
        )
        .unwrap();
        std::fs::write(
            package.join("build.zig.zon"),
            format!(
                ".{{\n    .name = .sample_binding,\n    .version = \"0.0.0\",\n    \
                 .fingerprint = 0x{fingerprint:016x},\n    \
                 .minimum_zig_version = \"0.16.0\",\n    \
                 .paths = .{{ \"build.zig\", \"build.zig.zon\", \"src\" }},\n}}\n",
                fingerprint = package_fingerprint(b"sample_binding"),
            ),
        )
        .unwrap();
        std::fs::write(
            package.join("src/root.zig"),
            "extern fn fixture_value() callconv(.c) c_int;\n\npub fn value() c_int {\n    return fixture_value();\n}\n",
        )
        .unwrap();

        let session = ValidationSession {
            language: Language::Zig,
            working_directory: package.clone(),
            manifest: Some(package.join("build.zig")),
            fingerprint: "debug-only-ffi-project".into(),
            env: std::collections::BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: std::collections::BTreeMap::new(),
        };
        (directory, session)
    }

    /// The decisive check for the profile-mismatch fix: `alef build` with no `--release` leaves
    /// only `target/debug/`, and the scaffolded `build.zig`'s `ffi_path` default only ever
    /// searches `target/release/` -- without the override this fails at snippet validation time
    /// with exactly the toolchain error the bug report ("unable to find dynamic system library")
    /// describes. Runs a real `zig build` link against a real dynamic library, not a stubbed-out
    /// placeholder file. ~keep
    #[test]
    fn a_snippet_links_against_the_debug_profile_when_release_is_missing() {
        if which::which("zig").is_err() {
            return;
        }

        let (project, session) = debug_only_ffi_project();
        let snippet = zig_snippet(
            "const sample_binding = @import(\"sample_binding\");\n\npub fn main() void {\n    _ = sample_binding.value();\n}\n",
        );

        let (status, output) = ZigValidator
            .validate_in_session(
                &snippet,
                ValidationLevel::Compile,
                TOOLCHAIN_TEST_TIMEOUT_SECS,
                Some(&session),
            )
            .expect("session validates");

        assert_eq!(
            status,
            SnippetStatus::Pass,
            "must fall back to and link against the debug-profile library when release is missing: {output:?}"
        );
        drop(project);
    }

    #[test]
    fn session_include_paths_are_passed_to_zig() {
        let mut command = std::process::Command::new("zig");
        apply_include_paths(
            &mut command,
            &[
                std::path::PathBuf::from("include"),
                std::path::PathBuf::from("vendor/include"),
            ],
        );

        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            ["-I", "include", "-I", "vendor/include"]
        );
    }
}
