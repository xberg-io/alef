use crate::snippets::error::Result;
use crate::snippets::scratch::ScratchDir;
use crate::snippets::session::ValidationSession;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::snippets::validators::{BatchValidation, SnippetValidator, run_command};

pub struct PythonValidator;
const PYREFLY_UNAVAILABLE: &str = "pyrefly is not available for Python type-checking";

const BATCH_FILE_PREFIX: &str = "snippet_batch_";
const BATCH_CHECKER_NAME: &str = "alef_batch_check.py";
const BATCH_AST_MODE: &str = "ast";
const BATCH_COMPILE_MODE: &str = "compile";
const BATCH_FAILED_WITHOUT_DIAGNOSTIC: &str = "the Python batch checker failed without a per-snippet diagnostic";
const BATCH_RUN_UNSUPPORTED: &str = "Python batch validation does not cover the run level";

/// One interpreter start for the whole batch instead of one per snippet: the checker walks every
/// file it is handed and emits one JSON line per file, so a syntax error in one snippet neither
/// aborts the run nor leaks into another snippet's result. It always exits 0, so the exit status
/// carries no per-snippet meaning and a missing line is what marks a snippet unjudged. ~keep
const BATCH_CHECKER_SOURCE: &str = r#"import ast
import json
import sys

mode = sys.argv[1]
for path in sys.argv[2:]:
    result = {"path": path, "ok": True, "error": ""}
    try:
        with open(path, encoding="utf-8") as handle:
            source = handle.read()
        if mode == "ast":
            ast.parse(source, filename=path)
        else:
            compile(source, path, "exec")
    except (SyntaxError, ValueError, UnicodeDecodeError, OSError) as error:
        result["ok"] = False
        result["error"] = "{}: {}".format(type(error).__name__, error)
    print(json.dumps(result))
"#;

impl PythonValidator {
    fn validate_batch_with_context(
        snippets: &[&Snippet],
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<BatchValidation> {
        if level == ValidationLevel::TypeCheck && which::which("pyrefly").is_err() {
            return Ok(vec![
                (SnippetStatus::Unavailable, Some(PYREFLY_UNAVAILABLE.to_string()));
                snippets.len()
            ]);
        }
        let dir = match session {
            Some(session) => session.scratch_dir()?,
            None => ScratchDir::isolated()?,
        };
        let mut paths = Vec::with_capacity(snippets.len());
        let mut file_names = Vec::with_capacity(snippets.len());
        for (index, snippet) in snippets.iter().enumerate() {
            let file_name = format!("{BATCH_FILE_PREFIX}{index}.py");
            let path = dir.path().join(&file_name);
            std::fs::write(&path, Self::patch_code(&snippet.code))?;
            paths.push(path);
            file_names.push(file_name);
        }
        // `pyrefly`'s own parser is not contractually identical to CPython's: a construct it
        // tolerates or recovers from differently than `python -m py_compile` would (task #463 --
        // a published snippet with an empty `for` loop body, a hard `IndentationError` in real
        // Python) must still fail `typecheck`, not pass because `typecheck` never asked the real
        // interpreter at all. Runs the same `Compile`-level batch checker
        // `validate_batch_with_context` uses for `ValidationLevel::Compile`, ahead of `pyrefly`,
        // and overrides any snippet it fails -- see `apply_compile_precheck`. ~keep
        let compile_precheck = if level == ValidationLevel::TypeCheck {
            Some(Self::batch_compile_precheck(
                &file_names,
                dir.path(),
                &paths,
                session,
                timeout_secs,
            )?)
        } else {
            None
        };
        let mut command = Self::batch_command(level, dir.path(), &paths)?;
        if let Some(session) = session {
            session.apply(&mut command);
            command.env("PYTHONPATH", &session.working_directory);
        }
        let (success, output) = run_command(&mut command, timeout_secs)?;
        let results = match level {
            ValidationLevel::TypeCheck => Self::typecheck_results(&file_names, success, &output),
            _ => Self::checker_results(&file_names, &output),
        };
        Ok(match compile_precheck {
            Some(compile_results) => Self::apply_compile_precheck(results, compile_results),
            None => results,
        })
    }

    /// Runs the `Compile`-level batch checker over the same source files a `TypeCheck` batch is
    /// about to hand to `pyrefly`, ahead of it -- see the `compile_precheck` comment in
    /// `validate_batch_with_context` (task #463).
    fn batch_compile_precheck(
        file_names: &[String],
        directory: &std::path::Path,
        paths: &[std::path::PathBuf],
        session: Option<&ValidationSession>,
        timeout_secs: u64,
    ) -> Result<BatchValidation> {
        let mut command = Self::batch_command(ValidationLevel::Compile, directory, paths)?;
        if let Some(session) = session {
            session.apply(&mut command);
            command.env("PYTHONPATH", &session.working_directory);
        }
        let (_success, output) = run_command(&mut command, timeout_secs)?;
        Ok(Self::checker_results(file_names, &output))
    }

    /// Overrides a `typecheck` result with the real interpreter's own `Compile`-level failure
    /// whenever the two disagree -- a file that does not even compile has nothing left for
    /// `pyrefly` to usefully say, and its own diagnostic (or silence) must never let the snippet
    /// through as `Pass`. Left untouched when the compile precheck passed: a real `pyrefly`
    /// finding (a genuine type error `py_compile` cannot see at all) must still fail on its own
    /// terms. ~keep
    fn apply_compile_precheck(mut results: BatchValidation, compile_results: BatchValidation) -> BatchValidation {
        for (result, compile_result) in results.iter_mut().zip(compile_results) {
            if compile_result.0 == SnippetStatus::Fail {
                *result = compile_result;
            }
        }
        results
    }

    fn batch_command(
        level: ValidationLevel,
        directory: &std::path::Path,
        paths: &[std::path::PathBuf],
    ) -> Result<std::process::Command> {
        if level == ValidationLevel::TypeCheck {
            let mut command = std::process::Command::new("pyrefly");
            command.arg("check").args(paths);
            return Ok(command);
        }
        let mode = match level {
            ValidationLevel::Syntax => BATCH_AST_MODE,
            ValidationLevel::Compile => BATCH_COMPILE_MODE,
            ValidationLevel::TypeCheck | ValidationLevel::Run => {
                return Err(crate::snippets::error::Error::Other(BATCH_RUN_UNSUPPORTED.to_string()));
            }
        };
        let checker_path = directory.join(BATCH_CHECKER_NAME);
        std::fs::write(&checker_path, BATCH_CHECKER_SOURCE)?;
        let mut command = std::process::Command::new(Self::interpreter());
        command.arg(&checker_path).arg(mode).args(paths);
        Ok(command)
    }

    /// Maps the batch checker's JSON lines back to the snippet that owns each file. A snippet the
    /// checker never reported on fails carrying the real output rather than passing by default —
    /// which is why the exit status is not consulted here: the checker reports every file it is
    /// given and exits 0 either way, so a missing line, not a non-zero exit, is what says a
    /// snippet went unjudged. ~keep
    fn checker_results(file_names: &[String], output: &str) -> BatchValidation {
        let mut reported = vec![None; file_names.len()];
        let mut unmatched = Vec::new();
        for line in output.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let entry = serde_json::from_str::<serde_json::Value>(line).ok();
            let index = entry
                .as_ref()
                .and_then(|value| value.get("path"))
                .and_then(serde_json::Value::as_str)
                .and_then(|path| Self::owner(file_names, path));
            match (index, entry) {
                (Some(index), Some(entry)) => {
                    let failed = entry.get("ok").and_then(serde_json::Value::as_bool) != Some(true);
                    let message = entry
                        .get("error")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    reported[index] = Some(if failed {
                        (SnippetStatus::Fail, Some(message))
                    } else {
                        (SnippetStatus::Pass, None)
                    });
                }
                _ => unmatched.push(line.to_string()),
            }
        }
        let fallback = Self::fallback_message(reported.iter().all(Option::is_some), &unmatched);
        Self::finalize(reported, fallback)
    }

    /// Attributes `pyrefly check` diagnostics back to each file. Its full-text blocks start with an
    /// `ERROR` line and name the file on the following `-->` line; the one-line `min-text` form
    /// carries the path on the `ERROR` line itself, so both shapes are read. ~keep
    fn typecheck_results(file_names: &[String], success: bool, output: &str) -> BatchValidation {
        let mut blocks: Vec<Vec<String>> = Vec::new();
        let mut unmatched = Vec::new();
        let mut current: Option<Vec<String>> = None;
        for line in output.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("ERROR") {
                blocks.extend(current.take());
                current = Some(vec![line.to_string()]);
            } else if trimmed.starts_with("INFO") {
                blocks.extend(current.take());
            } else if let Some(block) = current.as_mut() {
                block.push(line.to_string());
            } else if !trimmed.is_empty() {
                unmatched.push(line.to_string());
            }
        }
        blocks.extend(current.take());

        let mut diagnostics = vec![Vec::new(); file_names.len()];
        for block in blocks {
            match Self::block_owner(file_names, &block) {
                Some(index) => diagnostics[index].push(block.join("\n")),
                None => unmatched.push(block.join("\n")),
            }
        }
        let attributed = diagnostics.iter().any(|messages| !messages.is_empty());
        let fallback = Self::fallback_message(success || attributed, &unmatched);
        let reported = diagnostics
            .into_iter()
            .map(|messages| (!messages.is_empty()).then(|| (SnippetStatus::Fail, Some(messages.join("\n")))))
            .collect();
        Self::finalize(reported, fallback)
    }

    fn fallback_message(resolved: bool, unmatched: &[String]) -> Option<String> {
        (!resolved).then(|| {
            if unmatched.is_empty() {
                BATCH_FAILED_WITHOUT_DIAGNOSTIC.to_string()
            } else {
                unmatched.join("\n")
            }
        })
    }

    fn finalize(reported: Vec<Option<(SnippetStatus, Option<String>)>>, fallback: Option<String>) -> BatchValidation {
        reported
            .into_iter()
            .map(|value| match (value, &fallback) {
                (Some(value), _) => value,
                (None, Some(message)) => (SnippetStatus::Fail, Some(message.clone())),
                (None, None) => (SnippetStatus::Pass, None),
            })
            .collect()
    }

    fn block_owner(file_names: &[String], block: &[String]) -> Option<usize> {
        block.iter().find_map(|line| {
            let candidate = match line.split_once("--> ") {
                Some((_, rest)) => rest,
                None => line.trim_start().strip_prefix("ERROR ")?,
            };
            Self::owner(file_names, Self::strip_location_suffix(candidate.trim()))
        })
    }

    /// Cuts `pyrefly`'s trailing `:line:col` (and, in the one-line `min-text` form, everything after
    /// it) off a location. A Windows path opens with a drive prefix — `C:\Users\…\snippet_batch_1.py`
    /// — so cutting at the first colon yields `C`, which owns no file; an unattributed block on a
    /// failing run is then charged to every snippet, reporting the passing ones as broken. Only the
    /// colon that follows a single-letter drive is skipped. ~keep
    fn strip_location_suffix(candidate: &str) -> &str {
        let bytes = candidate.as_bytes();
        let drive = if bytes.len() > 1 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
            2
        } else {
            0
        };
        match candidate[drive..].find(':') {
            Some(offset) => &candidate[..drive + offset],
            None => candidate,
        }
    }

    /// Splits on both separators rather than going through `Path::file_name`: the paths parsed here
    /// come out of another process's diagnostics, so a `\`-separated path can reach a non-Windows
    /// build (and the reverse), and `Path` only honours the separators of the host it was compiled
    /// for. ~keep
    fn owner(file_names: &[String], path: &str) -> Option<usize> {
        let name = path.rsplit(['/', '\\']).next()?;
        file_names.iter().position(|file_name| file_name.as_str() == name)
    }

    fn interpreter() -> &'static str {
        if which::which("python3").is_ok() {
            "python3"
        } else {
            "python"
        }
    }

    fn validate_with_context(
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<(SnippetStatus, Option<String>)> {
        if level == ValidationLevel::TypeCheck && which::which("pyrefly").is_err() {
            return Ok((SnippetStatus::Unavailable, Some(PYREFLY_UNAVAILABLE.to_string())));
        }
        let dir = match session {
            Some(session) => session.scratch_dir()?,
            None => ScratchDir::isolated()?,
        };
        let code = Self::patch_code(&snippet.code);
        let snippet_path = dir.path().join("snippet.py");
        std::fs::write(&snippet_path, &code)?;
        let python = Self::interpreter();
        let path = snippet_path.to_string_lossy().to_string();
        if level == ValidationLevel::TypeCheck
            && let Some(failure) = Self::compile_precheck(python, dir.path(), &path, session, timeout_secs)?
        {
            return Ok(failure);
        }
        let mut command = Self::command(level, dir.path(), python, &path)?;
        if let Some(session) = session {
            session.apply(&mut command);
            command.env("PYTHONPATH", &session.working_directory);
        }
        let (success, output) = run_command(&mut command, timeout_secs)?;
        if success {
            Ok((SnippetStatus::Pass, None))
        } else {
            Ok((SnippetStatus::Fail, Some(output)))
        }
    }

    fn command(
        level: ValidationLevel,
        directory: &std::path::Path,
        python: &str,
        path: &str,
    ) -> Result<std::process::Command> {
        let command = match level {
            ValidationLevel::Syntax => {
                let checker_path = directory.join("check.py");
                std::fs::write(&checker_path, "import ast, sys\nast.parse(open(sys.argv[1]).read())\n")?;
                let mut command = std::process::Command::new(python);
                command.args([checker_path.to_string_lossy().as_ref(), path]);
                command
            }
            ValidationLevel::Compile => {
                let mut command = std::process::Command::new(python);
                command.args(["-m", "py_compile", path]);
                command
            }
            ValidationLevel::TypeCheck => {
                let mut command = std::process::Command::new("pyrefly");
                command.args(["check", path]);
                command
            }
            ValidationLevel::Run => {
                let mut command = std::process::Command::new(python);
                command.arg(path);
                command
            }
        };
        Ok(command)
    }

    /// The single-snippet analogue of `batch_compile_precheck` -- see the `compile_precheck`
    /// comment on `validate_batch_with_context` (task #463). Returns the real `py_compile`
    /// failure when the file does not compile, so `pyrefly` never even runs in that case; `None`
    /// when it compiles cleanly, letting the caller proceed to the requested `TypeCheck` command.
    fn compile_precheck(
        python: &str,
        directory: &std::path::Path,
        path: &str,
        session: Option<&ValidationSession>,
        timeout_secs: u64,
    ) -> Result<Option<(SnippetStatus, Option<String>)>> {
        let mut command = Self::command(ValidationLevel::Compile, directory, python, path)?;
        if let Some(session) = session {
            session.apply(&mut command);
            command.env("PYTHONPATH", &session.working_directory);
        }
        let (success, output) = run_command(&mut command, timeout_secs)?;
        Ok((!success).then_some((SnippetStatus::Fail, Some(output))))
    }

    fn patch_code(code: &str) -> String {
        let trimmed = code.trim();

        if trimmed.starts_with(' ') || trimmed.starts_with('\t') {
            let min_indent = trimmed
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(|line| line.len() - line.trim_start().len())
                .min()
                .unwrap_or(0);

            if min_indent > 0 {
                let dedented = trimmed
                    .lines()
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
                    .join("\n");

                return Self::patch_signatures(&dedented);
            }
        }

        Self::patch_signatures(code)
    }

    fn patch_signatures(code: &str) -> String {
        let lines: Vec<&str> = code.lines().collect();
        let mut output = Vec::new();
        let mut index = 0;

        while index < lines.len() {
            output.push(lines[index].to_string());
            let trimmed = lines[index].trim();
            let is_def_start =
                trimmed.starts_with("def ") || trimmed.starts_with("async def ") || trimmed.starts_with("class ");

            if is_def_start {
                let mut signature_end = index;
                let mut has_inline_body = false;

                while signature_end < lines.len() {
                    let trimmed_signature = lines[signature_end].trim();
                    if signature_end > index {
                        output.push(lines[signature_end].to_string());
                    }

                    if trimmed_signature.ends_with(':') {
                        break;
                    }

                    if let Some(arrow_pos) = trimmed_signature.find("->") {
                        let after_arrow = &trimmed_signature[arrow_pos + 2..];
                        if let Some(colon_pos) = after_arrow.find(':') {
                            let after_colon = after_arrow[colon_pos + 1..].trim();
                            if !after_colon.is_empty() {
                                has_inline_body = true;
                            }
                            break;
                        }

                        if signature_end == index {
                            let last = output.len() - 1;
                            output[last] = format!("{}:", lines[signature_end]);
                        }
                        break;
                    }

                    if trimmed_signature.contains("): ") || trimmed_signature.contains("):\t") {
                        has_inline_body = true;
                        break;
                    }

                    if trimmed_signature.ends_with(')') && signature_end > index {
                        let last = output.len() - 1;
                        output[last] = format!("{}:", output[last]);
                        break;
                    }

                    signature_end += 1;
                }

                if signature_end >= lines.len() {
                    let last = output.len() - 1;
                    if !output[last].trim().ends_with(':') {
                        output[last] = format!("{}:", output[last]);
                    }
                    let indent = lines[index].chars().take_while(|c| c.is_whitespace()).count();
                    output.push(format!("{}...", " ".repeat(indent + 4)));
                    index = signature_end;
                    continue;
                }

                if has_inline_body {
                    index = signature_end + 1;
                    continue;
                }

                let next_content = (signature_end + 1..lines.len())
                    .find(|candidate| !lines[*candidate].trim().is_empty())
                    .map(|candidate| lines[candidate]);

                let has_body = next_content.is_some_and(|line| line.starts_with(' ') || line.starts_with('\t'));
                if !has_body {
                    let last = output.len() - 1;
                    if !output[last].trim().ends_with(':') {
                        output[last] = format!("{}:", output[last]);
                    }

                    let indent = lines[index].chars().take_while(|c| c.is_whitespace()).count();
                    output.push(format!("{}...", " ".repeat(indent + 4)));
                }

                index = signature_end + 1;
                continue;
            }

            index += 1;
        }

        output.join("\n")
    }
}

impl SnippetValidator for PythonValidator {
    fn language(&self) -> Language {
        Language::Python
    }

    fn is_available(&self) -> bool {
        which::which("python3").is_ok() || which::which("python").is_ok()
    }

    fn is_available_at(&self, level: ValidationLevel) -> bool {
        if level != ValidationLevel::TypeCheck {
            return self.is_available();
        }
        which::which("pyrefly").is_ok()
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

    /// `Run` is declined: each snippet must execute in its own process so its output, exit status
    /// and side effects belong to it alone. ~keep
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

    fn supports_batching(&self) -> bool {
        true
    }

    fn max_level(&self) -> ValidationLevel {
        ValidationLevel::Run
    }

    fn is_dependency_error(&self, output: &str) -> bool {
        let _ = output;
        false
    }
}

#[cfg(test)]
#[path = "python_tests.rs"]
mod tests;
