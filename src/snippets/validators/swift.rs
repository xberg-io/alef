use crate::snippets::error::Result;
use crate::snippets::scratch::ScratchDir;
use crate::snippets::session::ValidationSession;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::snippets::validators::{SnippetValidator, run_command, run_command_streams};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct SwiftModuleLookup {
    package_root: std::path::PathBuf,
    environment: Vec<(String, String)>,
}

/// Where a snippet's `swiftc` invocation should look for the package's built modules.
///
/// Two lists, because SwiftPM's generated modulemaps are named per target rather than
/// `module.modulemap`, so clang cannot discover them from a search directory and they have to be
/// named outright. ~keep
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct SwiftModulePaths {
    search_directories: Vec<std::path::PathBuf>,
    module_maps: Vec<std::path::PathBuf>,
}

#[derive(Default)]
pub struct SwiftValidator {
    module_directories: Mutex<HashMap<SwiftModuleLookup, std::result::Result<SwiftModulePaths, String>>>,
}

impl SnippetValidator for SwiftValidator {
    fn language(&self) -> Language {
        Language::Swift
    }

    fn is_available(&self) -> bool {
        which::which("swiftc").is_ok()
    }

    fn validate(
        &self,
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
    ) -> Result<(SnippetStatus, Option<String>)> {
        let dir = ScratchDir::isolated()?;
        let file = dir.path().join("snippet.swift");
        std::fs::write(&file, snippet.code.trim())?;

        let mut command = std::process::Command::new("swiftc");
        match level {
            ValidationLevel::Syntax => {
                command.args(["-parse"]).arg(&file);
            }
            ValidationLevel::Compile => {
                let out = dir.path().join("snippet");
                command.args(["-o"]).arg(&out).arg(&file);
            }
            ValidationLevel::TypeCheck => {
                command.args(["-typecheck", "-warnings-as-errors"]).arg(&file);
            }
            ValidationLevel::Run => {
                let out = dir.path().join("snippet");
                command.args(["-o"]).arg(&out).arg(&file);
            }
        }

        let (success, output) = run_command(&mut command, timeout_secs)?;
        if success {
            Ok((SnippetStatus::Pass, None))
        } else {
            Ok((SnippetStatus::Fail, Some(output)))
        }
    }

    fn max_level(&self) -> ValidationLevel {
        ValidationLevel::TypeCheck
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
        let file = dir.path().join("snippet.swift");
        std::fs::write(&file, snippet.code.trim())?;
        let module_paths = self.module_directories(session, timeout_secs)?;
        let mut command = std::process::Command::new("swiftc");
        match level {
            ValidationLevel::Syntax => {
                command.arg("-parse");
            }
            ValidationLevel::TypeCheck => {
                command.args(["-typecheck", "-warnings-as-errors"]);
            }
            ValidationLevel::Compile => {
                command.arg("-typecheck");
            }
            ValidationLevel::Run => {
                command.arg("-o").arg(dir.path().join("snippet"));
            }
        }
        for directory in &module_paths.search_directories {
            command.arg("-I").arg(directory);
        }
        for module_map in &module_paths.module_maps {
            command
                .arg("-Xcc")
                .arg(format!("-fmodule-map-file={}", module_map.display()));
        }
        if let Some(binary_directory) = module_paths.search_directories.first().and_then(|path| path.parent()) {
            command.arg("-L").arg(binary_directory);
        }
        command.arg(&file);
        session.apply(&mut command);
        let (success, output) = run_command(&mut command, timeout_secs)?;
        Ok(if success {
            (SnippetStatus::Pass, None)
        } else {
            (SnippetStatus::Fail, Some(output))
        })
    }

    /// ~keep `cannot find 'x' in scope` fires for a name the generated snippet failed to bind
    /// just as readily as for one a missing module would have supplied, so it is the same
    /// ambiguous shape task #130 rejected for `TS2304` — and an accepted match is rewritten into
    /// `Unavailable` by `runner::finalize_result`, taking a real defect out of the failure tally.
    /// `no such module` names the unbuilt artifact itself and is unambiguous.
    fn is_dependency_error(&self, output: &str) -> bool {
        output.contains("no such module")
    }
}

impl SwiftValidator {
    fn module_directories(&self, session: &ValidationSession, timeout_secs: u64) -> Result<SwiftModulePaths> {
        self.cached_module_directories(session, || swift_module_directories(session, timeout_secs))
    }

    fn cached_module_directories(
        &self,
        session: &ValidationSession,
        resolve: impl FnOnce() -> Result<SwiftModulePaths>,
    ) -> Result<SwiftModulePaths> {
        let lookup = swift_module_lookup(session);
        let mut cache = self.module_directories.lock().map_err(|error| {
            crate::snippets::error::Error::Other(format!("locking Swift module-directory cache: {error}"))
        })?;
        if let Some(cached) = cache.get(&lookup) {
            return cached.clone().map_err(crate::snippets::error::Error::Other);
        }
        // ~keep Hold the lock while SwiftPM resolves the path: releasing it here lets every
        // parallel snippet observe the same miss and launch its own `swift build --show-bin-path`.
        let resolved = resolve().map_err(|error| error.to_string());
        cache.insert(lookup, resolved.clone());
        resolved.map_err(crate::snippets::error::Error::Other)
    }
}

fn swift_module_lookup(session: &ValidationSession) -> SwiftModuleLookup {
    let package_root = session
        .manifest
        .as_deref()
        .and_then(std::path::Path::parent)
        .unwrap_or(&session.working_directory)
        .to_path_buf();
    SwiftModuleLookup {
        package_root,
        environment: session
            .env
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
    }
}

/// `swift build --show-bin-path` is not a lookup: SwiftPM resolves the package first, which can
/// fetch dependencies over the network and, on a package it cannot resolve, retry until something
/// gives up. It ran unbounded here while every other subprocess in snippet validation was already
/// under the session's `timeout_secs`; it is now under the same bound, and the same process-group
/// teardown, as the `swiftc` invocation it feeds. ~keep
fn swift_module_directories(session: &ValidationSession, timeout_secs: u64) -> Result<SwiftModulePaths> {
    let mut command = std::process::Command::new("swift");
    command.args(["build", "--show-bin-path"]);
    session.apply(&mut command);
    if let Some(package_root) = session.manifest.as_deref().and_then(std::path::Path::parent) {
        command.current_dir(package_root);
    }
    let captured = run_command_streams(&mut command, timeout_secs)?;
    if !captured.success {
        return Err(crate::snippets::error::Error::Other(
            crate::snippets::diagnostics::bounded_text(captured.stderr.trim()),
        ));
    }
    let binary_directory = std::path::PathBuf::from(captured.stdout.trim());
    swift_module_directories_in(&binary_directory)
}

/// SwiftPM has two build systems and they lay the products out differently, so a snippet has to
/// compile against whichever one built the package.
///
/// The *native* system writes the `.swiftmodule` files into a `Modules/` directory beside the
/// binaries and generates a `module.modulemap` per C target under the bin path, where `-I` finds
/// it. Swift 6.3 made *swiftbuild* the default: it puts the `.swiftmodule` files directly in the
/// bin path, creates no `Modules/` directory at all, and writes generated modulemaps to
/// `Intermediates.noindex/GeneratedModuleMaps-<platform>/<Target>.modulemap` -- named per target
/// rather than `module.modulemap`, so clang cannot discover them from a search directory and they
/// have to be named outright with `-fmodule-map-file`.
///
/// Looking for both shapes is what keeps this working across toolchains; each degrades to nothing
/// on the layout that does not use it. Before this, a package built by swiftbuild resolved no
/// modules at all and every snippet failed with `no such module`, which `is_dependency_error`
/// classifies as Unavailable -- so a whole language reported as "environment not ready" rather
/// than failing, and `--strict` blocked the run with nothing to fix in the tree. ~keep
fn swift_module_directories_in(binary_directory: &std::path::Path) -> Result<SwiftModulePaths> {
    let mut paths = SwiftModulePaths {
        // `Modules` first: `validate_in_session` takes `-L` from this entry's parent, which is the
        // bin path under either layout. ~keep
        search_directories: vec![binary_directory.join("Modules"), binary_directory.to_path_buf()],
        module_maps: generated_module_maps_for(binary_directory),
    };
    let Ok(entries) = std::fs::read_dir(binary_directory) else {
        return Ok(paths);
    };
    for entry in entries {
        let path = entry?.path();
        if path.join("module.modulemap").is_file() {
            paths.search_directories.push(path);
        } else if path.join("include/module.modulemap").is_file() {
            // ~keep `-I` resolves a modulemap only in the directory named, so the nested case has
            // to name `include` itself. The parent stays on the list because a target laid out
            // this way still serves its headers from it.
            paths.search_directories.push(path.join("include"));
            paths.search_directories.push(path);
        }
    }
    Ok(paths)
}

/// swiftbuild keeps generated modulemaps under the scratch directory's `Intermediates.noindex`,
/// a sibling of the `Products` tree the bin path points into. Walk up from the bin path to find
/// it rather than rebuilding a scratch path from `--scratch-path`: the bin path is the only
/// location SwiftPM is actually asked for, so it is the only one guaranteed to be right. ~keep
fn generated_module_maps_for(binary_directory: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut module_maps = Vec::new();
    for ancestor in binary_directory.ancestors() {
        let intermediates = ancestor.join("Intermediates.noindex");
        let Ok(entries) = std::fs::read_dir(&intermediates) else {
            continue;
        };
        for entry in entries.flatten() {
            let generated = entry.path();
            let is_generated_module_maps = generated
                .file_name()
                .and_then(std::ffi::OsStr::to_str)
                .is_some_and(|name| name.starts_with("GeneratedModuleMaps"));
            if !is_generated_module_maps {
                continue;
            }
            let Ok(candidates) = std::fs::read_dir(&generated) else {
                continue;
            };
            for candidate in candidates.flatten() {
                let module_map = candidate.path();
                if module_map.extension().is_some_and(|extension| extension == "modulemap") {
                    module_maps.push(module_map);
                }
            }
        }
        break;
    }
    // Stable ordering keeps the `swiftc` command line reproducible across runs. ~keep
    module_maps.sort();
    module_maps
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Swift is deliberately not batched. `swiftc` compiles one module per invocation, and a
    /// module accepts top-level code in exactly one file: handing it several snippets at once
    /// fails every one of them with `expressions are not allowed at the top level` before any
    /// snippet's own code is judged, which the per-snippet path never does. There is no way to
    /// scope a snippet's top-level statements into a namespace of its own, so the batch hook stays
    /// declined and the runner keeps using one process per snippet. ~keep
    #[test]
    fn batching_is_declined_because_one_module_cannot_hold_two_top_level_snippets() {
        let first = swift_snippet("print(\"one\")\n");
        let second = swift_snippet("print(\"two\")\n");

        let validator = SwiftValidator::default();
        assert!(!validator.supports_batching());
        for level in [
            ValidationLevel::Syntax,
            ValidationLevel::Compile,
            ValidationLevel::TypeCheck,
            ValidationLevel::Run,
        ] {
            let declined = validator.validate_batch_in_session(&[&first, &second], level, 10, None);
            assert!(
                declined.is_none(),
                "{level:?} must fall back to one process per snippet"
            );
        }
    }

    #[test]
    fn a_session_resolves_its_swiftpm_module_directories_once() {
        let validator = SwiftValidator::default();
        let first_session = ValidationSession {
            language: Language::Swift,
            working_directory: PathBuf::from("package"),
            manifest: Some(PathBuf::from("package/Package.swift")),
            fingerprint: "first-target-identity".into(),
            env: Default::default(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: Default::default(),
        };
        let mut second_session = first_session.clone();
        second_session.fingerprint = "second-target-identity".into();
        let invocations = AtomicUsize::new(0);
        let expected = SwiftModulePaths {
            search_directories: vec![PathBuf::from(".build/debug/Modules")],
            module_maps: Vec::new(),
        };

        for session in [&first_session, &second_session] {
            let resolved = validator
                .cached_module_directories(session, || {
                    invocations.fetch_add(1, Ordering::SeqCst);
                    Ok(expected.clone())
                })
                .expect("module directories resolve");
            assert_eq!(resolved, expected);
        }

        assert_eq!(invocations.load(Ordering::SeqCst), 1);
    }

    fn swift_snippet(code: &str) -> crate::snippets::types::Snippet {
        crate::snippets::types::Snippet {
            id: None,
            path: std::path::PathBuf::from("snippet.swift"),
            language: Language::Swift,
            title: None,
            code: code.into(),
            start_line: 1,
            block_index: 0,
            annotation: None,
            metadata: crate::snippets::types::SnippetMetadata::default(),
            source_origin: crate::snippets::types::SourceOrigin {
                path: std::path::PathBuf::from("snippet.swift"),
                line: 1,
                block_index: 0,
            },
        }
    }

    #[test]
    fn missing_swiftpm_bin_directory_is_not_an_io_error() {
        let directory = tempfile::tempdir().expect("temp directory");
        let missing = directory.path().join("not-built");

        assert_eq!(
            swift_module_directories_in(&missing).expect("missing bin directory is tolerated"),
            SwiftModulePaths {
                search_directories: vec![missing.join("Modules"), missing.clone()],
                module_maps: Vec::new(),
            }
        );
    }

    /// The layout `swift build` produces from Swift 6.3 on, where the default build system became
    /// swiftbuild: `.swiftmodule` files directly in the bin path, no `Modules/` directory, and
    /// generated modulemaps named per target two levels up under `Intermediates.noindex`.
    /// Reproduced against `swift:6.4`; before this the bin path contributed no `-I` of its own and
    /// the modulemaps were never passed, so nothing resolved. ~keep
    #[test]
    fn swiftbuild_layout_contributes_the_bin_path_and_its_generated_module_maps() {
        let directory = tempfile::tempdir().expect("temp directory");
        let out = directory.path().join("out");
        let binary_directory = out.join("Products/Debug-linux-aarch64");
        let generated = out.join("Intermediates.noindex/GeneratedModuleMaps-linux-aarch64");
        std::fs::create_dir_all(&binary_directory).expect("bin directory");
        std::fs::create_dir_all(&generated).expect("generated modulemap directory");
        std::fs::write(binary_directory.join("SampleFixture.swiftmodule"), "").expect("module");
        std::fs::write(generated.join("RustBridgeC.modulemap"), "module RustBridgeC {}").expect("modulemap");
        std::fs::write(generated.join("ignored.txt"), "not a modulemap").expect("decoy");

        let resolved = swift_module_directories_in(&binary_directory).expect("layout resolves");

        assert_eq!(
            resolved.search_directories,
            vec![binary_directory.join("Modules"), binary_directory.clone()]
        );
        assert_eq!(resolved.module_maps, vec![generated.join("RustBridgeC.modulemap")]);
    }

    /// The native layout must keep resolving exactly as it did, since a package built by the older
    /// build system is still the common case on pinned toolchains. ~keep
    #[test]
    fn native_layout_still_resolves_a_c_targets_module_map_directory() {
        let directory = tempfile::tempdir().expect("temp directory");
        let binary_directory = directory.path().join("debug");
        let c_target = binary_directory.join("RustBridgeC.build");
        std::fs::create_dir_all(&c_target).expect("c target directory");
        std::fs::create_dir_all(binary_directory.join("Modules")).expect("modules directory");
        std::fs::write(c_target.join("module.modulemap"), "module RustBridgeC {}").expect("modulemap");

        let resolved = swift_module_directories_in(&binary_directory).expect("layout resolves");

        assert!(resolved.search_directories.contains(&c_target));
        assert_eq!(
            resolved.search_directories.first(),
            Some(&binary_directory.join("Modules"))
        );
        assert!(resolved.module_maps.is_empty());
    }
}
