#[path = "node_project_root.rs"]
mod node_project_root;

use crate::snippets::error::Result;
use crate::snippets::session::ValidationSession;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::snippets::validators::{
    BatchValidation, SnippetValidation, SnippetValidator, all_error_lines_match, run_command,
};
use node_project_root::resolve_isolated_scratch;
use std::io::Write;

pub struct TypeScriptValidator;

const SNIPPET_FILE_NAME: &str = "snippet.ts";
const BATCH_FILE_PREFIX: &str = "snippet_batch_";

/// A file whose top level contains an `export` is a module, and a module's top-level names are
/// scoped to it. Without this every batched file shares one global script scope, so two snippets
/// each declaring `const result` collide with TS2451 — a failure the one-process-per-snippet path
/// could never produce. ~keep
const MODULE_SCOPE_MARKER: &str = "export {};";

const UNRESOLVED_BATCH_SLOT: &str = "TypeScript batch validation produced no result for this snippet";
const BATCH_FAILED_WITHOUT_DIAGNOSTIC: &str = "tsc failed without a snippet-specific diagnostic";

/// Every docs/e2e-generated TypeScript snippet that reads a fixture file from disk (see
/// `typescript/docs_file_expression.jinja`, `typescript/docs_file_assignment.jinja`) does it via
/// `await import("node:fs/promises")`, unconditionally, regardless of which binding target the
/// snippet demonstrates. When the target session's own package has no `@types/node` reachable
/// (a WASM/browser package has no reason to depend on Node's types at all), `tsc` does not report
/// the usual "cannot find module" -- for a `node:`-prefixed specifier it cannot resolve, it
/// degrades the dynamic `import(...)` to a bare identifier lookup and reports
/// `TS2591: Cannot find name 'node:fs/promises'`, the Node-specific sibling of TS2580. Since alef
/// itself is what emits this construct into every TypeScript target uniformly, alef -- not the
/// consumer's package.json -- is responsible for making it typecheck. Declaring only the single
/// function these templates actually call keeps this from ever needing to match a real
/// `@types/node`'s full surface; TypeScript merges declaration blocks for the same module
/// specifier across files, so this coexists with a real `@types/node` if one is also resolvable
/// (verified: a second, richer `declare module "node:fs/promises"` in the same program does not
/// conflict with this one). Written unconditionally on every check, session or not -- an unused
/// ambient module declaration is a no-op, so detecting whether a given snippet's code actually
/// needs it would only add a second, fragile way for this to miss a case.
///
/// The same gap exists for the bare `Buffer` global: `ts_bytes_value_expression`'s base64 branch
/// (`src/e2e/codegen/typescript/test_file/bytes.rs`) emits `Buffer.from(value, "base64")` into
/// every TypeScript target uniformly, the same way the file-path branch emits the dynamic
/// `node:fs/promises` import above. Without `@types/node` resolvable, `tsc` reports
/// `TS2591: Cannot find name 'Buffer'. Do you need to install type definitions for node?` --
/// `Buffer` is on TypeScript's own curated list of Node globals that earn this specific hint
/// (`require`, `process`, `__dirname`, ... alongside it), the sibling case to the degraded dynamic
/// import above rather than a different bug. Declared here for the same reason: alef is what
/// emits the construct, so alef makes it typecheck, with only the one static method these
/// templates actually call. ~keep
const NODE_AMBIENT_DECLARATION_FILE_NAME: &str = "alef_node_fs_promises_ambient.d.ts";
const NODE_AMBIENT_DECLARATION_CONTENT: &str = "declare module \"node:fs/promises\" {\n  function readFile(path: \
                                                 string): Promise<Uint8Array>;\n}\n\ndeclare const Buffer: {\n  \
                                                 from(data: string, encoding: string): Uint8Array;\n};\n";

impl TypeScriptValidator {
    fn validate_batch_with_context(
        snippets: &[&Snippet],
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<BatchValidation> {
        let mut results: Vec<Option<SnippetValidation>> = vec![None; snippets.len()];
        let mut checked = Vec::new();
        for (index, snippet) in snippets.iter().enumerate() {
            if Self::is_trivially_valid(&snippet.code) {
                results[index] = Some((SnippetStatus::Pass, None));
            } else {
                checked.push(index);
            }
        }
        if !checked.is_empty() {
            let file_names = checked
                .iter()
                .map(|index| format!("{BATCH_FILE_PREFIX}{index}.ts"))
                .collect::<Vec<_>>();
            let outcomes = Self::check_batch(snippets, &checked, &file_names, level, timeout_secs, session)?;
            for (index, outcome) in checked.into_iter().zip(outcomes) {
                results[index] = Some(outcome);
            }
        }
        Ok(results
            .into_iter()
            .map(|value| value.unwrap_or_else(|| (SnippetStatus::Error, Some(UNRESOLVED_BATCH_SLOT.to_string()))))
            .collect())
    }

    fn check_batch(
        snippets: &[&Snippet],
        checked: &[usize],
        file_names: &[String],
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<BatchValidation> {
        // Anchored on the first checked snippet's own real file: an unclaimed batch groups
        // whatever snippets share no configured session, which in practice share one doc tree and
        // therefore one real project root -- see `node_project_root` for why only a real,
        // on-disk path can resolve to one at all. ~keep
        let anchor = checked.first().map(|&index| snippets[index].path.as_path());
        let temporary_directory = match session {
            Some(_) => None,
            None => Some(resolve_isolated_scratch(anchor, timeout_secs)?),
        };
        let directory = match (session, temporary_directory.as_ref()) {
            (Some(value), _) => value.workspace_directory()?,
            (None, Some((value, _))) => value.path().to_path_buf(),
            (None, None) => unreachable!(),
        };
        if session.is_none() {
            let types_node_resolved = temporary_directory.as_ref().is_some_and(|(_, resolved)| *resolved);
            std::fs::write(
                directory.join("tsconfig.json"),
                Self::isolated_tsconfig(types_node_resolved),
            )?;
        }
        std::fs::write(
            directory.join(NODE_AMBIENT_DECLARATION_FILE_NAME),
            NODE_AMBIENT_DECLARATION_CONTENT,
        )?;
        for (index, file_name) in checked.iter().zip(file_names) {
            let code = Self::as_module(&Self::dedent(&snippets[*index].code));
            std::fs::write(directory.join(file_name), code)?;
        }
        let project = session
            .and_then(|value| value.manifest.as_ref())
            .map(|manifest| Self::write_overlay_config_for(&directory, manifest, file_names))
            .transpose()?;
        let mut command = Self::check_command(level, &directory, project.as_deref());
        if let Some(session) = session {
            session.apply(&mut command);
        }
        let (success, output) = run_command(&mut command, timeout_secs)?;
        Ok(Self::batch_results(file_names, success, &output))
    }

    /// Attributes `tsc --pretty false` diagnostics — `path/to/file.ts(line,col): error TSxxxx: …` —
    /// back to the snippet that owns each file. Continuation lines of a message chain carry no path
    /// of their own, so they stay with the diagnostic they follow. ~keep
    fn batch_results(file_names: &[String], success: bool, output: &str) -> BatchValidation {
        let mut diagnostics = vec![Vec::new(); file_names.len()];
        let mut unmatched = Vec::new();
        let mut current: Option<usize> = None;
        for line in output.lines() {
            if line.trim().is_empty() {
                continue;
            }
            match Self::diagnostic_owner(file_names, line) {
                Some(index) => {
                    diagnostics[index].push(line.to_string());
                    current = Some(index);
                }
                None if line.starts_with([' ', '\t']) => match current {
                    Some(index) => diagnostics[index].push(line.to_string()),
                    None => unmatched.push(line.to_string()),
                },
                None => {
                    current = None;
                    unmatched.push(line.to_string());
                }
            }
        }
        let attributed = diagnostics.iter().any(|messages| !messages.is_empty());
        let fallback = (!success && !attributed).then(|| {
            if unmatched.is_empty() {
                BATCH_FAILED_WITHOUT_DIAGNOSTIC.to_string()
            } else {
                unmatched.join("\n")
            }
        });
        diagnostics
            .into_iter()
            .map(|messages| match (messages.is_empty(), &fallback) {
                (true, Some(message)) => (SnippetStatus::Fail, Some(message.clone())),
                (true, None) => (SnippetStatus::Pass, None),
                (false, _) => (SnippetStatus::Fail, Some(messages.join("\n"))),
            })
            .collect()
    }

    fn diagnostic_owner(file_names: &[String], line: &str) -> Option<usize> {
        let (path, _) = line.split_once('(')?;
        let name = std::path::Path::new(path).file_name()?;
        file_names
            .iter()
            .position(|file_name| std::ffi::OsStr::new(file_name.as_str()) == name)
    }

    fn as_module(code: &str) -> String {
        format!("{}\n{MODULE_SCOPE_MARKER}\n", code.trim_end())
    }

    fn is_trivially_valid(code: &str) -> bool {
        Self::is_api_signature(code) || code.trim().starts_with("!!!") || code.trim().starts_with("???")
    }

    fn validate_with_context(
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<(SnippetStatus, Option<String>)> {
        if Self::is_trivially_valid(&snippet.code) {
            return Ok((SnippetStatus::Pass, None));
        }
        let temporary_directory = match session {
            Some(_) => None,
            None => Some(resolve_isolated_scratch(Some(snippet.path.as_path()), timeout_secs)?),
        };
        let directory = match (session, temporary_directory.as_ref()) {
            (Some(value), _) => value.workspace_directory()?,
            (None, Some((value, _))) => value.path().to_path_buf(),
            (None, None) => unreachable!(),
        };
        if session.is_none() {
            let types_node_resolved = temporary_directory.as_ref().is_some_and(|(_, resolved)| *resolved);
            std::fs::write(
                directory.join("tsconfig.json"),
                Self::isolated_tsconfig(types_node_resolved),
            )?;
        }
        std::fs::write(
            directory.join(NODE_AMBIENT_DECLARATION_FILE_NAME),
            NODE_AMBIENT_DECLARATION_CONTENT,
        )?;
        let file_path = directory.join(SNIPPET_FILE_NAME);
        let mut file = std::fs::File::create(&file_path)?;
        file.write_all(Self::dedent(&snippet.code).as_bytes())?;
        let project = session
            .and_then(|value| value.manifest.as_ref())
            .map(|manifest| Self::write_overlay_config(&directory, manifest))
            .transpose()?;
        let mut command = Self::command(level, &file_path, &directory, session, project.as_deref());
        let (success, output) = run_command(&mut command, timeout_secs)?;
        Ok(if success {
            (SnippetStatus::Pass, None)
        } else {
            (SnippetStatus::Fail, Some(output))
        })
    }

    /// `include_types_node` must be `true` only when `node_project_root::resolve_isolated_scratch`
    /// has already confirmed a real ancestor `@types/node` install -- naming `"node"` in `types`
    /// when nothing resolves fails with `TS2688: Cannot find type definition file for 'node'`
    /// (confirmed by hand-probing `tsc`), the exact failure this mechanism exists to prevent. The
    /// unset-`"types"` branch deliberately does *not* rely on automatic type-acquisition to fill
    /// the gap when `false`: see `node_project_root`'s module docs for why that default is not
    /// something alef can depend on being implemented the same way across `tsc` generations. ~keep
    fn isolated_tsconfig(include_types_node: bool) -> &'static str {
        if include_types_node {
            r#"{"compilerOptions":{"strict":true,"noEmit":true,"target":"ES2022","module":"ES2022","moduleResolution":"bundler","skipLibCheck":true,"types":["node"]},"include":["*.ts"]}"#
        } else {
            r#"{"compilerOptions":{"strict":true,"noEmit":true,"target":"ES2022","module":"ES2022","moduleResolution":"bundler","skipLibCheck":true},"include":["*.ts"]}"#
        }
    }

    fn write_overlay_config(directory: &std::path::Path, manifest: &std::path::Path) -> Result<std::path::PathBuf> {
        Self::write_overlay_config_for(directory, manifest, &[SNIPPET_FILE_NAME.to_string()])
    }

    fn write_overlay_config_for(
        directory: &std::path::Path,
        manifest: &std::path::Path,
        file_names: &[String],
    ) -> Result<std::path::PathBuf> {
        let path = directory.join("tsconfig.json");
        let manifest_value: serde_json::Value = serde_json::from_slice(&std::fs::read(manifest)?).map_err(|error| {
            crate::snippets::error::Error::Other(format!(
                "parsing TypeScript package manifest {}: {error}",
                manifest.display()
            ))
        })?;
        // Unlike `isolated_tsconfig`'s glob `include`, these two overlays name every checked file
        // explicitly in `files` -- the ambient declaration must be listed the same way, or `tsc`
        // never loads it and `NODE_AMBIENT_DECLARATION_FILE_NAME` sits on disk unused. ~keep
        let file_names_with_ambient: Vec<String> = file_names
            .iter()
            .cloned()
            .chain(std::iter::once(NODE_AMBIENT_DECLARATION_FILE_NAME.to_string()))
            .collect();
        let content = if manifest_value.get("compilerOptions").is_some() {
            Self::project_overlay(directory, manifest, &file_names_with_ambient)
        } else {
            Self::package_overlay(manifest, &manifest_value, &file_names_with_ambient)?
        };
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&content).map_err(|error| {
                crate::snippets::error::Error::Other(format!("serializing TypeScript snippet config: {error}"))
            })?,
        )?;
        Ok(path)
    }

    fn project_overlay(
        directory: &std::path::Path,
        manifest: &std::path::Path,
        file_names: &[String],
    ) -> serde_json::Value {
        let files = file_names.iter().map(|name| directory.join(name)).collect::<Vec<_>>();
        serde_json::json!({
            "extends": manifest,
            "compilerOptions": { "noEmit": true },
            "files": files
        })
    }

    fn package_overlay(
        manifest: &std::path::Path,
        manifest_value: &serde_json::Value,
        file_names: &[String],
    ) -> Result<serde_json::Value> {
        let package_name = manifest_value
            .get("name")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                crate::snippets::error::Error::Other(format!("no package name in {}", manifest.display()))
            })?;
        let package_root = manifest.parent().unwrap_or_else(|| std::path::Path::new("."));
        let declaration = manifest_value
            .get("types")
            .or_else(|| manifest_value.get("typings"))
            .and_then(serde_json::Value::as_str)
            .map(|entry| package_root.join(entry))
            .unwrap_or_else(|| package_root.to_path_buf());
        Ok(serde_json::json!({
            "compilerOptions": {
                "strict": true,
                "noEmit": true,
                "target": "ES2022",
                "module": "ES2022",
                "moduleResolution": "bundler",
                "skipLibCheck": true,
                "paths": { package_name: [declaration] }
            },
            "files": file_names
        }))
    }

    fn command(
        level: ValidationLevel,
        file_path: &std::path::Path,
        isolated_directory: &std::path::Path,
        session: Option<&ValidationSession>,
        project: Option<&std::path::Path>,
    ) -> std::process::Command {
        let mut command = if level == ValidationLevel::Run {
            let mut command = std::process::Command::new("tsx");
            if let Some(project) = project {
                command.args(["--tsconfig", project.to_string_lossy().as_ref()]);
            }
            command.arg(file_path);
            command
        } else {
            Self::check_command(level, isolated_directory, project)
        };
        if let Some(session) = session {
            session.apply(&mut command);
        }
        command
    }

    fn check_command(
        level: ValidationLevel,
        isolated_directory: &std::path::Path,
        project: Option<&std::path::Path>,
    ) -> std::process::Command {
        let mut command = std::process::Command::new("tsc");
        command.args(["--noEmit", "--pretty", "false"]);
        if level == ValidationLevel::Syntax {
            command.arg("--noCheck");
        }
        if let Some(project) = project {
            command.args(["--project", project.to_string_lossy().as_ref()]);
        } else {
            command.current_dir(isolated_directory);
        }
        command
    }

    fn dedent(code: &str) -> String {
        let min_indent = code
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| line.len() - line.trim_start().len())
            .min()
            .unwrap_or(0);

        if min_indent == 0 {
            return code.to_string();
        }

        code.lines()
            .map(|line| {
                if line.trim().is_empty() {
                    String::new()
                } else if line.len() > min_indent {
                    line[min_indent..].to_string()
                } else {
                    line.trim().to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn is_api_signature(code: &str) -> bool {
        let trimmed = code.trim();

        if trimmed.lines().count() <= 6 {
            let has_fn_decl = trimmed.starts_with("function ")
                || trimmed.starts_with("async function ")
                || trimmed.starts_with("export function ")
                || trimmed.starts_with("export async function ");
            return has_fn_decl && !trimmed.contains('{');
        }

        false
    }
}

impl SnippetValidator for TypeScriptValidator {
    fn language(&self) -> Language {
        Language::TypeScript
    }

    fn is_available(&self) -> bool {
        which::which("tsc").is_ok()
    }

    fn validate(
        &self,
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
    ) -> Result<(SnippetStatus, Option<String>)> {
        Self::validate_with_context(snippet, level, timeout_secs, None)
    }

    fn validate_in_session(
        &self,
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<(SnippetStatus, Option<String>)> {
        Self::validate_with_context(snippet, level, timeout_secs, session)
    }

    /// `Run` is declined: `tsx` executes one file and its stdout belongs to that snippet alone, so
    /// there is nothing to attribute back across a batch. ~keep
    fn validate_batch_in_session(
        &self,
        snippets: &[&Snippet],
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Option<Result<BatchValidation>> {
        (level != ValidationLevel::Run)
            .then(|| Self::validate_batch_with_context(snippets, level, timeout_secs, session))
    }

    fn requires_session_exclusivity(&self) -> bool {
        true
    }

    fn supports_batching(&self) -> bool {
        true
    }

    fn max_level(&self) -> ValidationLevel {
        ValidationLevel::Run
    }

    fn missing_session_artifacts(
        &self,
        session: &ValidationSession,
        _level: ValidationLevel,
    ) -> Vec<std::path::PathBuf> {
        crate::snippets::validators::session_artifacts::missing_typescript_declaration(session)
    }

    // Only codes tsc emits when it could not *locate* a module, namespace, or declaration file
    // -- the shape that actually means "this run's environment lacks a dependency or build
    // artifact." Ordinary type errors (TS2322 not-assignable, TS2345 argument mismatch, TS2339
    // missing property, TS2304/TS2552 unresolved name, TS7006 implicit any, TS1005/TS1128 syntax,
    // TS18046/TS18047/TS2531/TS2532 nullability, TS2451 redeclaration, ...) report a real defect
    // in the snippet or its own types and must stay `Fail` with the compiler's own text, not get
    // relabeled as an environment problem the reader is told `alef build` will fix. The previous,
    // much broader list caught genuine TS2322/TS2304 type errors under the "missing dependency or
    // build artifact" caption, which sent the reader to rebuild toolchains for a defect no rebuild
    // could fix -- see task #130. When a run's error lines don't unanimously match this narrow
    // set, `finalize_result` leaves `status` as `Fail` and the message as the compiler's own
    // output verbatim, so an unrecognized failure is always shown, never re-captioned by guess.
    // ~keep
    fn is_dependency_error(&self, output: &str) -> bool {
        let patterns = [
            "TS2307", // Cannot find module 'X' or its corresponding type declarations.
            "TS2305", // Module 'X' has no exported member 'Y' -- stale/missing build artifact.
            "TS2306", // File 'X' is not a module.
            "TS7016", // Could not find a declaration file for module 'X'.
            "TS2792", // Cannot find module 'X'. Did you mean to set the 'moduleResolution' option?
            "TS2503", // Cannot find namespace 'X'.
            "TS2580", // Cannot find name 'X'. Do you need to install type definitions for it?
        ];

        all_error_lines_match(
            output,
            |line| line.contains("error TS"),
            |line| patterns.iter().any(|pattern| line.contains(pattern)),
        )
    }
}

/// Whether `tsc` runs, not merely resolves: a version-manager shim (e.g. nvm) spawns fine then
/// exits non-zero, so a PATH-only check leaves the skip below unreachable and fires the assert
/// everywhere TypeScript is absent. Shared by this file's own `mod tests` and by the sibling
/// `*_tsc_tests.rs` modules declared below, which all gate real-`tsc` tests the same way. ~keep
#[cfg(test)]
fn tsc_is_runnable() -> bool {
    static RUNNABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *RUNNABLE.get_or_init(|| {
        std::process::Command::new("tsc")
            .arg("--version")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snippets::types::{SnippetMetadata, SourceOrigin};
    use std::collections::BTreeMap;

    const TOOLCHAIN_TEST_TIMEOUT_SECS: u64 = 120;

    #[test]
    fn package_manifest_maps_local_declarations() {
        let package = tempfile::tempdir().unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let manifest = package.path().join("package.json");
        std::fs::write(&manifest, r#"{"name":"sample-binding","types":"index.d.ts"}"#).unwrap();
        let config = TypeScriptValidator::write_overlay_config(scratch.path(), &manifest).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&std::fs::read(config).unwrap()).unwrap();
        assert_eq!(
            value["compilerOptions"]["paths"]["sample-binding"][0],
            package.path().join("index.d.ts").to_string_lossy().as_ref()
        );
        assert!(value["compilerOptions"].get("baseUrl").is_none());
    }

    /// `check_batch`/`validate_with_context` write `NODE_AMBIENT_DECLARATION_FILE_NAME` to disk
    /// unconditionally, but `tsc` only loads a file that also appears in the config's `files`
    /// list -- this is the other half, proving the package-manifest overlay branch names it. See
    /// `project_overlay_includes_the_node_ambient_declaration` for the other overlay branch, and
    /// `node_fs_promises_typechecks_without_a_types_node_dependency` for the end-to-end proof. ~keep
    #[test]
    fn package_overlay_includes_the_node_ambient_declaration() {
        let package = tempfile::tempdir().unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let manifest = package.path().join("package.json");
        std::fs::write(&manifest, r#"{"name":"sample-binding","types":"index.d.ts"}"#).unwrap();
        let config = TypeScriptValidator::write_overlay_config(scratch.path(), &manifest).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&std::fs::read(config).unwrap()).unwrap();
        let files = value["files"].as_array().expect("files array");
        assert!(
            files.iter().any(|entry| entry == NODE_AMBIENT_DECLARATION_FILE_NAME),
            "package overlay must list the node ambient declaration: {files:?}"
        );
    }

    /// Text-pinning proof for the piece that never needs a real `tsc` run: the ambient
    /// declaration's *content*, not just the fact that it is listed in `files`
    /// (`package_overlay_includes_the_node_ambient_declaration` /
    /// `project_overlay_includes_the_node_ambient_declaration` already cover that half). This only
    /// proves the declaration text alef writes to disk includes a `Buffer` global -- it does not
    /// prove `tsc` accepts it; see `buffer_from_base64_typechecks_without_a_types_node_dependency`
    /// in `node_ambient_declaration_tsc_tests.rs` for the real-compiler proof. ~keep
    #[test]
    fn node_ambient_declaration_content_declares_the_buffer_global() {
        assert!(
            NODE_AMBIENT_DECLARATION_CONTENT.contains("declare const Buffer"),
            "the ambient declaration must cover the bare `Buffer` global `ts_bytes_value_expression`'s base64 \
             branch emits, the same way it already covers `node:fs/promises`: {NODE_AMBIENT_DECLARATION_CONTENT:?}"
        );
    }

    /// Text-pinning proof for the conditional half of `isolated_tsconfig`: it must name `"node"`
    /// in `types` only when the caller has already confirmed a real ancestor `@types/node`
    /// install. Naming it unconditionally reproduces `TS2688: Cannot find type definition file for
    /// 'node'` for every session-less check that never resolves one (confirmed by hand-probing
    /// `tsc` against exactly that tsconfig shape with nothing on its `typeRoots` search path); see
    /// `an_uncovered_node_builtin_typechecks_when_the_real_project_has_types_node_installed` and
    /// `an_uncovered_node_builtin_fails_loudly_with_no_resolvable_project` in
    /// `node_project_root_tsc_tests.rs` for the real-compiler proof of both branches. ~keep
    #[test]
    fn isolated_tsconfig_names_node_types_only_when_told_to() {
        assert!(
            !TypeScriptValidator::isolated_tsconfig(false).contains("\"types\""),
            "the default isolated tsconfig must not name any `types` entry: {}",
            TypeScriptValidator::isolated_tsconfig(false)
        );
        assert!(
            TypeScriptValidator::isolated_tsconfig(true).contains("\"types\":[\"node\"]"),
            "the resolved-ancestor isolated tsconfig must explicitly name `\"node\"` in `types`: {}",
            TypeScriptValidator::isolated_tsconfig(true)
        );
    }

    /// The `project_overlay` branch (manifest is itself a tsconfig with `compilerOptions`) builds
    /// its `files` list differently -- absolute paths joined against `directory` -- so it needs
    /// its own proof the ambient declaration survives that branch too.
    #[test]
    fn project_overlay_includes_the_node_ambient_declaration() {
        let project = tempfile::tempdir().unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let manifest = project.path().join("tsconfig.json");
        std::fs::write(&manifest, r#"{"compilerOptions":{"strict":true}}"#).unwrap();
        let config = TypeScriptValidator::write_overlay_config(scratch.path(), &manifest).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&std::fs::read(config).unwrap()).unwrap();
        let files = value["files"].as_array().expect("files array");
        let ambient_path = scratch.path().join(NODE_AMBIENT_DECLARATION_FILE_NAME);
        assert!(
            files
                .iter()
                .any(|entry| entry.as_str() == Some(ambient_path.to_string_lossy().as_ref())),
            "project overlay must list the node ambient declaration: {files:?}"
        );
    }

    #[test]
    fn project_manifest_resolves_declared_local_package_and_replaces_stale_source() {
        if !super::tsc_is_runnable() {
            return;
        }
        let project = tempfile::tempdir().unwrap();
        let package = project.path().join("node_modules/sample-binding");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::write(
            package.join("package.json"),
            r#"{"name":"sample-binding","types":"index.d.ts"}"#,
        )
        .unwrap();
        std::fs::write(package.join("index.d.ts"), "export declare const value: number;\n").unwrap();
        let manifest = project.path().join("tsconfig.json");
        std::fs::write(
            &manifest,
            r#"{"compilerOptions":{"strict":true,"moduleResolution":"bundler","module":"ES2022"}}"#,
        )
        .unwrap();
        let session = ValidationSession {
            language: Language::TypeScript,
            working_directory: project.path().to_path_buf(),
            manifest: Some(manifest),
            fingerprint: "neutral-project".into(),
            env: BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        };

        let valid = snippet("import { value } from 'sample-binding';\nconst result: number = value;");
        let invalid = snippet("import { value } from 'sample-binding';\nconst result: string = value;");
        let (first, _) =
            TypeScriptValidator::validate_with_context(&valid, ValidationLevel::TypeCheck, 30, Some(&session)).unwrap();
        let (second, _) =
            TypeScriptValidator::validate_with_context(&invalid, ValidationLevel::TypeCheck, 30, Some(&session))
                .unwrap();

        assert_eq!(first, SnippetStatus::Pass);
        assert_eq!(second, SnippetStatus::Fail);
    }

    /// The batch overlay must carry the session manifest's own compiler options, or every batched
    /// snippet fails on an unresolved import of the generated bindings instead of on its own code. ~keep
    #[test]
    fn batch_in_a_session_resolves_the_local_package_the_manifest_declares() {
        if !super::tsc_is_runnable() {
            return;
        }
        let project = tempfile::tempdir().unwrap();
        let package = project.path().join("node_modules/sample-binding");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::write(
            package.join("package.json"),
            r#"{"name":"sample-binding","types":"index.d.ts"}"#,
        )
        .unwrap();
        std::fs::write(package.join("index.d.ts"), "export declare const value: number;\n").unwrap();
        let manifest = project.path().join("tsconfig.json");
        std::fs::write(
            &manifest,
            r#"{"compilerOptions":{"strict":true,"moduleResolution":"bundler","module":"ES2022"}}"#,
        )
        .unwrap();
        let session = ValidationSession {
            language: Language::TypeScript,
            working_directory: project.path().to_path_buf(),
            manifest: Some(manifest),
            fingerprint: "neutral-project".into(),
            env: BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        };
        let valid =
            snippet("import { value } from 'sample-binding';\nconst result: number = value;\nconsole.log(result);");
        let invalid =
            snippet("import { value } from 'sample-binding';\nconst result: string = value;\nconsole.log(result);");

        let results = TypeScriptValidator::validate_batch_with_context(
            &[&valid, &invalid, &valid],
            ValidationLevel::TypeCheck,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            Some(&session),
        )
        .expect("batch validation runs");

        assert_eq!(results.len(), 3);
        assert_eq!(results[0], (SnippetStatus::Pass, None), "{:?}", results[0]);
        assert_eq!(results[1].0, SnippetStatus::Fail);
        assert_eq!(results[2], (SnippetStatus::Pass, None), "{:?}", results[2]);
    }

    #[test]
    fn batch_declines_run_because_each_snippet_owns_its_own_output() {
        let only = snippet("console.log(1);");

        let declined = TypeScriptValidator.validate_batch_in_session(&[&only], ValidationLevel::Run, 30, None);

        assert!(declined.is_none());
    }

    #[test]
    fn batch_returns_one_result_per_snippet_in_input_order() {
        if !super::tsc_is_runnable() {
            return;
        }
        let first = snippet("const first: number = 1;\nconsole.log(first);");
        let second = snippet("const second: string = 'two';\nconsole.log(second);");
        let third = snippet("const third: boolean = true;\nconsole.log(third);");

        let results = TypeScriptValidator::validate_batch_with_context(
            &[&first, &second, &third],
            ValidationLevel::TypeCheck,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            None,
        )
        .expect("batch validation runs");

        assert_eq!(results.len(), 3);
        assert_eq!(results[0], (SnippetStatus::Pass, None));
        assert_eq!(results[1], (SnippetStatus::Pass, None));
        assert_eq!(results[2], (SnippetStatus::Pass, None));
    }

    #[test]
    fn batch_fails_only_the_broken_snippet_and_passes_its_neighbours() {
        if !super::tsc_is_runnable() {
            return;
        }
        let first = snippet("const value: number = 1;\nconsole.log(value);");
        let broken = snippet("const value: string = 2;\nconsole.log(value);");
        let third = snippet("const value: boolean = true;\nconsole.log(value);");

        let results = TypeScriptValidator::validate_batch_with_context(
            &[&first, &broken, &third],
            ValidationLevel::TypeCheck,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            None,
        )
        .expect("batch validation runs");

        assert_eq!(results.len(), 3);
        assert_eq!(results[0], (SnippetStatus::Pass, None), "{:?}", results[0]);
        assert_eq!(results[1].0, SnippetStatus::Fail);
        assert!(
            results[1]
                .1
                .as_deref()
                .is_some_and(|message| message.contains("TS2322")),
            "the failing snippet must carry its own diagnostic: {:?}",
            results[1].1
        );
        assert_eq!(results[2], (SnippetStatus::Pass, None), "{:?}", results[2]);
    }

    /// Every batched file lands in one tsc project, where top-level `const` declarations share a
    /// global script scope unless each file is a module. Two snippets both naming `result` would
    /// then both fail with TS2451 — a failure neither has when validated alone. ~keep
    #[test]
    fn batch_does_not_invent_redeclaration_failures_for_snippets_sharing_a_name() {
        if !super::tsc_is_runnable() {
            return;
        }
        let first = snippet("const result: number = 1;\nconsole.log(result);");
        let second = snippet("const result: number = 2;\nconsole.log(result);");

        let results = TypeScriptValidator::validate_batch_with_context(
            &[&first, &second],
            ValidationLevel::TypeCheck,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            None,
        )
        .expect("batch validation runs");

        assert_eq!(results, vec![(SnippetStatus::Pass, None), (SnippetStatus::Pass, None)]);
    }

    #[test]
    fn batch_passes_signature_only_snippets_without_compiling_them() {
        let signature = snippet("export function build(name: string): Promise<number>");
        let placeholder = snippet("!!! note\n    see the guide");

        let results = TypeScriptValidator::validate_batch_with_context(
            &[&signature, &placeholder],
            ValidationLevel::Syntax,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            None,
        )
        .expect("batch validation runs");

        assert_eq!(results, vec![(SnippetStatus::Pass, None), (SnippetStatus::Pass, None)]);
    }

    #[test]
    fn diagnostics_attach_to_the_file_named_at_the_start_of_the_line() {
        let file_names = vec!["snippet_batch_0.ts".to_string(), "snippet_batch_1.ts".to_string()];
        let output = "snippet_batch_1.ts(1,7): error TS2322: Type 'number' is not assignable to type 'string'.\n";

        let results = TypeScriptValidator::batch_results(&file_names, false, output);

        assert_eq!(results[0], (SnippetStatus::Pass, None));
        assert_eq!(results[1].0, SnippetStatus::Fail);
        assert_eq!(
            results[1].1.as_deref(),
            Some("snippet_batch_1.ts(1,7): error TS2322: Type 'number' is not assignable to type 'string'.")
        );
    }

    /// A compiler that fails for a reason no file owns — a bad tsconfig, a missing library — must
    /// fail every snippet carrying the real output, never silently pass them all. ~keep
    #[test]
    fn a_project_wide_failure_fails_every_snippet_with_the_real_output() {
        let file_names = vec!["snippet_batch_0.ts".to_string(), "snippet_batch_1.ts".to_string()];
        let output = "error TS5083: Cannot read file 'tsconfig.json'.\n";

        let results = TypeScriptValidator::batch_results(&file_names, false, output);

        assert_eq!(results.len(), 2);
        for result in &results {
            assert_eq!(result.0, SnippetStatus::Fail);
            assert_eq!(
                result.1.as_deref(),
                Some("error TS5083: Cannot read file 'tsconfig.json'.")
            );
        }
    }

    #[test]
    fn message_chain_continuation_lines_stay_with_their_diagnostic() {
        let file_names = vec!["snippet_batch_0.ts".to_string(), "snippet_batch_1.ts".to_string()];
        let output = "snippet_batch_0.ts(2,1): error TS2345: Argument mismatch.\n  Types of property 'id' differ.\n";

        let results = TypeScriptValidator::batch_results(&file_names, false, output);

        assert_eq!(
            results[0].1.as_deref(),
            Some("snippet_batch_0.ts(2,1): error TS2345: Argument mismatch.\n  Types of property 'id' differ.")
        );
        assert_eq!(results[1], (SnippetStatus::Pass, None));
    }

    /// task #130: a genuine type error (TS2322 "not assignable") must never be classified as a
    /// missing dependency -- the reader was told to `run alef build first` for a defect no
    /// rebuild could fix. ~keep
    #[test]
    fn is_dependency_error_rejects_a_type_mismatch() {
        let output = "snippet.ts(1,7): error TS2322: Type 'number' is not assignable to type 'string'.\n";

        assert!(
            !TypeScriptValidator.is_dependency_error(output),
            "TS2322 is a type error, not a missing dependency: {output:?}"
        );
    }

    /// task #130: TS2304 "cannot find name" is ambiguous -- it fires just as often for a typo or
    /// an undefined local as for a missing import, so it must not be guessed as a dependency
    /// failure either. ~keep
    #[test]
    fn is_dependency_error_rejects_an_unresolved_name() {
        let output = "snippet.ts(1,1): error TS2304: Cannot find name 'totallyUndefinedLocal'.\n";

        assert!(
            !TypeScriptValidator.is_dependency_error(output),
            "TS2304 is ambiguous and must not be classified as a missing dependency: {output:?}"
        );
    }

    /// A genuinely missing dependency (an import tsc could not locate at all) must still
    /// classify as one -- narrowing the pattern set must not overcorrect into treating every
    /// tsc failure as a snippet defect. ~keep
    #[test]
    fn is_dependency_error_accepts_an_unresolved_module() {
        let output = "snippet.ts(1,1): error TS2307: Cannot find module 'missing-package' or its corresponding \
                       type declarations.\n";

        assert!(
            TypeScriptValidator.is_dependency_error(output),
            "TS2307 is an unresolved module and must classify as a missing dependency: {output:?}"
        );
    }

    /// A missing `.d.ts` (the classic "toolchain built, artifact not generated yet" shape) must
    /// still classify as a missing dependency. ~keep
    #[test]
    fn is_dependency_error_accepts_a_missing_declaration_file() {
        let output = "snippet.ts(1,1): error TS7016: Could not find a declaration file for module 'binding-pkg'.\n";

        assert!(
            TypeScriptValidator.is_dependency_error(output),
            "TS7016 is a missing declaration file and must classify as a missing dependency: {output:?}"
        );
    }

    /// A mix of an unresolved module and a genuine type error is not confidently a dependency
    /// failure end to end -- `is_dependency_error` requires every error line to match, so this
    /// stays `Fail` with the raw compiler text rather than being relabeled. ~keep
    #[test]
    fn is_dependency_error_declines_a_mixed_batch() {
        let output = "snippet_batch_0.ts(1,1): error TS2307: Cannot find module 'missing-package'.\n\
                       snippet_batch_1.ts(2,7): error TS2322: Type 'number' is not assignable to type 'string'.\n";

        assert!(
            !TypeScriptValidator.is_dependency_error(output),
            "a run mixing a real type error must not be classified as a missing dependency: {output:?}"
        );
    }

    fn snippet(code: &str) -> Snippet {
        Snippet {
            id: None,
            path: "snippet.ts".into(),
            language: Language::TypeScript,
            title: None,
            code: code.into(),
            start_line: 1,
            block_index: 0,
            annotation: None,
            metadata: SnippetMetadata::default(),
            source_origin: SourceOrigin {
                path: "snippet.ts".into(),
                line: 1,
                block_index: 0,
            },
        }
    }
}

#[cfg(test)]
#[path = "wasm_optional_chain_tsc_tests.rs"]
mod wasm_optional_chain_tsc_tests;

#[cfg(test)]
#[path = "node_ambient_declaration_tsc_tests.rs"]
mod node_ambient_declaration_tsc_tests;

#[cfg(test)]
#[path = "node_project_root_tsc_tests.rs"]
mod node_project_root_tsc_tests;
