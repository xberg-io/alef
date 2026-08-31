use crate::snippets::cache::ValidationCache;
use crate::snippets::error::Result;
use crate::snippets::scratch::ScratchDir;
use crate::snippets::session::ValidationSession;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::snippets::validators::{BatchValidation, SnippetValidator, all_error_lines_match, run_command};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Namespace prefix that isolates one batched snippet's declarations from every other snippet's.
const BATCH_NAMESPACE_PREFIX: &str = "AlefSnippet";

/// Subdirectory of the session workspace holding the batch project. It is a fixed name rather than
/// a fresh temporary directory so `obj/`, `bin/` and the restored NuGet graph survive between runs;
/// its `.cs` files are cleared before every batch instead, because the .NET SDK globs `**/*.cs` and
/// a previous run's leftover source would otherwise compile into this run's project. ~keep
const BATCH_PROJECT_DIRECTORY: &str = "csharp-batch";

/// The wrapper type each snippet's statements are lifted into. Safe to share across snippets
/// because every snippet gets its own namespace.
const BATCH_WRAPPER_CLASS: &str = "Snippet";

/// MSBuild diagnostic headers, as `<path>(<line>,<column>): <severity> CSxxxx: <message>`.
const DIAGNOSTIC_MARKERS: [(&str, DiagnosticSeverity); 2] = [
    (": error ", DiagnosticSeverity::Error),
    (": warning ", DiagnosticSeverity::Warning),
];

const TYPE_KEYWORDS: [&str; 6] = ["class ", "struct ", "record ", "interface ", "enum ", "delegate "];

const DECLARATION_MODIFIERS: [&str; 12] = [
    "public ",
    "internal ",
    "private ",
    "protected ",
    "static ",
    "sealed ",
    "abstract ",
    "partial ",
    "readonly ",
    "unsafe ",
    "file ",
    "ref ",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum DiagnosticSeverity {
    Error,
    Warning,
}

struct CsharpDiagnostic {
    path: String,
    severity: DiagnosticSeverity,
    text: String,
}

/// One snippet's compilation unit inside a batch. File names are unique per snippet, so the file
/// name is the attribution token.
struct CsharpBatchUnit {
    file_name: String,
    source: String,
}

pub struct CsharpValidator;

impl CsharpValidator {
    fn validate_with_context(
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<(SnippetStatus, Option<String>)> {
        let temporary_directory = session.is_none().then(ScratchDir::isolated).transpose()?;
        let directory = match (session, temporary_directory.as_ref()) {
            (Some(value), _) => value.workspace_directory()?,
            (None, Some(value)) => value.path().to_path_buf(),
            (None, None) => unreachable!(),
        };
        let project_path = directory.join("Snippet.csproj");
        std::fs::write(&project_path, Self::project_source(session, "Exe")?)?;
        std::fs::write(directory.join("Program.cs"), Self::wrap_if_fragment(&snippet.code))?;
        let mut command = std::process::Command::new("dotnet");
        match level {
            ValidationLevel::Syntax | ValidationLevel::Compile => {
                command.args(["build", "--nologo", "-v", "quiet"]);
            }
            ValidationLevel::TypeCheck => {
                command.args(["build", "--nologo", "-v", "quiet", "-warnaserror"]);
            }
            ValidationLevel::Run => {
                command.args(["run", "--nologo"]);
            }
        }
        command.current_dir(&directory);
        Self::configure_dotnet_environment(&mut command, &directory)?;
        if let Some(session) = session {
            session.apply_environment(&mut command);
        }
        let (success, output) = run_command(&mut command, timeout_secs)?;
        Ok(if success {
            (SnippetStatus::Pass, None)
        } else {
            (SnippetStatus::Fail, Some(output))
        })
    }

    fn project_source(session: Option<&ValidationSession>, output_type: &str) -> Result<String> {
        let target_framework = session
            .and_then(|value| value.manifest.as_deref())
            .map(Self::target_framework)
            .transpose()?
            .unwrap_or_else(|| "net8.0".to_owned());
        let reference = session
            .and_then(|value| value.manifest.as_ref())
            .map_or_else(String::new, |manifest| {
                format!(
                    "  <ItemGroup><ProjectReference Include={:?} /></ItemGroup>\n",
                    manifest.to_string_lossy()
                )
            });
        Ok(format!(
            "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><OutputType>{output_type}</OutputType><TargetFramework>{target_framework}</TargetFramework><Nullable>enable</Nullable><ImplicitUsings>enable</ImplicitUsings></PropertyGroup>{reference}</Project>\n"
        ))
    }

    fn validate_batch_with_context(
        snippets: &[&Snippet],
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Option<Result<BatchValidation>> {
        if level == ValidationLevel::Run {
            return None;
        }
        let units = Self::plan_batch(snippets, level, session)?;
        Some(Self::compile_batch(&units, level, timeout_secs, session))
    }

    /// Builds one compilation unit per snippet, or declines the batch.
    ///
    /// C# permits top-level statements in exactly one compilation unit per project, so N
    /// statement-shaped snippets cannot share a project as written. Each snippet's statements are
    /// therefore lifted into its own namespaced wrapper class, and any type declarations trailing
    /// them stay at namespace scope where C# requires them.
    ///
    /// A snippet that declares its own namespace keeps it, because rewriting one would break the
    /// qualified references inside it. Two such snippets sharing a namespace can genuinely
    /// redeclare each other's types, so the batch is declined and the runner falls back to one
    /// process per snippet, which cannot collide at all. ~keep
    fn plan_batch(
        snippets: &[&Snippet],
        level: ValidationLevel,
        session: Option<&ValidationSession>,
    ) -> Option<Vec<CsharpBatchUnit>> {
        let fingerprint = session.map(|value| value.fingerprint.as_str());
        let mut declared_namespaces: HashMap<String, String> = HashMap::new();
        let mut units = Vec::with_capacity(snippets.len());
        for snippet in snippets {
            let code = Self::wrap_if_fragment(&snippet.code);
            let identifier = ValidationCache::key(snippet, level, fingerprint);
            let source = match Self::declared_namespace(&code) {
                Some(namespace) => {
                    let owner = declared_namespaces
                        .entry(namespace)
                        .or_insert_with(|| identifier.clone());
                    if owner != &identifier {
                        return None;
                    }
                    format!("{}\n", code.trim_end())
                }
                None => Self::batch_source(&code, &format!("{BATCH_NAMESPACE_PREFIX}{identifier}")),
            };
            units.push(CsharpBatchUnit {
                file_name: format!("Snippet_{identifier}.cs"),
                source,
            });
        }
        Some(units)
    }

    fn batch_source(code: &str, namespace: &str) -> String {
        let (directives, body) = Self::split_using_directives(code);
        let (statements, declarations) = Self::split_type_declarations(&body);
        let signature = if statements.contains("await ") {
            "async System.Threading.Tasks.Task RunAsync(string[] args)"
        } else {
            "void Run(string[] args)"
        };
        let mut source = String::new();
        if !directives.trim().is_empty() {
            source.push_str(directives.trim_end());
            source.push_str("\n\n");
        }
        source.push_str(&format!("namespace {namespace};\n\n"));
        source.push_str(&format!(
            "internal static class {BATCH_WRAPPER_CLASS}\n{{\n    internal static {signature}\n    {{\n"
        ));
        source.push_str(statements.trim_end());
        source.push_str("\n    }\n}\n");
        if !declarations.trim().is_empty() {
            source.push('\n');
            source.push_str(declarations.trim_end());
            source.push('\n');
        }
        source
    }

    fn compile_batch(
        units: &[CsharpBatchUnit],
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<BatchValidation> {
        let temporary_directory = session.is_none().then(ScratchDir::isolated).transpose()?;
        let root = match (session, temporary_directory.as_ref()) {
            (Some(value), _) => value.workspace_directory()?,
            (None, Some(value)) => value.path().to_path_buf(),
            (None, None) => unreachable!(),
        };
        let directory = root.join(BATCH_PROJECT_DIRECTORY);
        std::fs::create_dir_all(&directory)?;
        Self::clear_batch_sources(&directory)?;
        std::fs::write(
            directory.join("Snippet.csproj"),
            Self::project_source(session, "Library")?,
        )?;
        let mut written = HashSet::new();
        for unit in units {
            if written.insert(unit.file_name.clone()) {
                std::fs::write(directory.join(&unit.file_name), &unit.source)?;
            }
        }
        let mut command = std::process::Command::new("dotnet");
        command.args(["build", "--nologo", "-v", "quiet"]);
        if level == ValidationLevel::TypeCheck {
            command.arg("-warnaserror");
        }
        command.current_dir(&directory);
        Self::configure_dotnet_environment(&mut command, &root)?;
        if let Some(session) = session {
            session.apply_environment(&mut command);
        }
        let (success, output) = run_command(&mut command, timeout_secs)?;
        Ok(Self::batch_results(
            units,
            level == ValidationLevel::TypeCheck,
            success,
            &output,
        ))
    }

    /// Removes the previous batch's sources. The .NET SDK compiles every `.cs` it globs under the
    /// project directory, so a leftover file would be a phantom snippet in this run's results. ~keep
    fn clear_batch_sources(directory: &Path) -> Result<()> {
        let entries = std::fs::read_dir(directory).map_err(|error| {
            crate::snippets::error::Error::Other(format!(
                "reading C# snippet batch directory {}: {error}",
                directory.display()
            ))
        })?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|extension| extension == "cs")
                && let Err(error) = std::fs::remove_file(&path)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                return Err(crate::snippets::error::Error::Other(format!(
                    "removing stale C# snippet batch source {}: {error}",
                    path.display()
                )));
            }
        }
        Ok(())
    }

    fn batch_results(units: &[CsharpBatchUnit], warnings_fail: bool, success: bool, output: &str) -> BatchValidation {
        let mut attributed = vec![Vec::<String>::new(); units.len()];
        for diagnostic in Self::parse_diagnostics(output) {
            if diagnostic.severity == DiagnosticSeverity::Warning && !warnings_fail {
                continue;
            }
            for owner in Self::owning_units(units, &diagnostic.path) {
                if !attributed[owner].contains(&diagnostic.text) {
                    attributed[owner].push(diagnostic.text.clone());
                }
            }
        }
        let attributed_any = attributed.iter().any(|messages| !messages.is_empty());
        let fallback = (!success && !attributed_any).then(|| {
            let trimmed = output.trim();
            if trimmed.is_empty() {
                "dotnet build failed without a snippet-specific diagnostic".to_string()
            } else {
                trimmed.to_string()
            }
        });
        attributed
            .into_iter()
            .map(|messages| match (messages.is_empty(), &fallback) {
                (true, Some(message)) => (SnippetStatus::Fail, Some(message.clone())),
                (true, None) => (SnippetStatus::Pass, None),
                (false, _) => (SnippetStatus::Fail, Some(messages.join("\n"))),
            })
            .collect()
    }

    /// MSBuild diagnostics are single-line and are printed twice — inline and again in the
    /// `Build FAILED.` summary — so callers deduplicate rather than parse blocks. ~keep
    fn parse_diagnostics(output: &str) -> Vec<CsharpDiagnostic> {
        output
            .lines()
            .filter_map(|line| {
                let (path, severity) = Self::diagnostic_header(line)?;
                Some(CsharpDiagnostic {
                    path: path.to_owned(),
                    severity,
                    text: line.trim_end().to_owned(),
                })
            })
            .collect()
    }

    fn diagnostic_header(line: &str) -> Option<(&str, DiagnosticSeverity)> {
        for (marker, severity) in DIAGNOSTIC_MARKERS {
            let Some(index) = line.find(marker) else {
                continue;
            };
            let head = line[..index].trim_end();
            let Some(position) = head.strip_suffix(')').and_then(|head| head.rfind('(')) else {
                continue;
            };
            if position > 0 {
                return Some((&head[..position], severity));
            }
        }
        None
    }

    fn owning_units(units: &[CsharpBatchUnit], path: &str) -> Vec<usize> {
        let Some(name) = Path::new(path).file_name().and_then(std::ffi::OsStr::to_str) else {
            return Vec::new();
        };
        units
            .iter()
            .enumerate()
            .filter(|(_, unit)| unit.file_name == name)
            .map(|(index, _)| index)
            .collect()
    }

    fn declared_namespace(code: &str) -> Option<String> {
        code.lines().find_map(|line| {
            if line.starts_with(char::is_whitespace) {
                return None;
            }
            let rest = line.trim().strip_prefix("namespace ")?;
            Some(rest.trim().trim_end_matches([';', '{']).trim().to_owned())
        })
    }

    fn split_using_directives(code: &str) -> (String, String) {
        let mut directives = Vec::new();
        let mut body = Vec::new();
        let mut past_directives = false;
        for line in code.lines() {
            let trimmed = line.trim();
            let leading = trimmed.is_empty() || trimmed.starts_with("//") || Self::is_using_directive(trimmed);
            if !past_directives && leading {
                directives.push(line);
            } else {
                past_directives = true;
                body.push(line);
            }
        }
        (directives.join("\n"), body.join("\n"))
    }

    /// A `using` *directive* (`using System;`), as opposed to a `using` *statement* or declaration
    /// (`using (var stream = ...)`, `using var stream = ...;`) — only the directive may be hoisted
    /// above the snippet's wrapper class. ~keep
    fn is_using_directive(trimmed: &str) -> bool {
        let rest = trimmed
            .strip_prefix("global using ")
            .or_else(|| trimmed.strip_prefix("using "));
        rest.is_some_and(|rest| trimmed.ends_with(';') && !rest.starts_with("var ") && !rest.starts_with('('))
    }

    /// Splits a snippet body at the first top-level type declaration. C# allows type declarations
    /// only *after* top-level statements, so everything from that point on belongs at namespace
    /// scope rather than inside the wrapper method — a type cannot be declared in a method body.
    /// Split a snippet body into its top-level statements and the type declarations that follow.
    ///
    /// The split point is found by brace depth, not by column. Requiring a declaration to start at
    /// column zero looked equivalent and is not: alef's own C# snippet generator indents a trailing
    /// `sealed class` by four spaces, so the declaration was left inside the wrapper method, where
    /// C# does not allow one -- 54 of one consumer's 283 snippets failed on `CS1513: } expected`.
    /// Depth is what the guard was actually reaching for, since the case it must not match is a
    /// `class` nested inside a method body. ~keep
    fn split_type_declarations(body: &str) -> (String, String) {
        let mut depth = 0usize;
        for (index, line) in body.lines().enumerate() {
            if depth == 0 && Self::is_type_declaration(line) {
                let lines: Vec<&str> = body.lines().collect();
                return (lines[..index].join("\n"), lines[index..].join("\n"));
            }
            depth = Self::brace_depth_after(line, depth);
        }
        (body.to_owned(), String::new())
    }

    /// Brace depth after `line`, ignoring braces inside string and character literals.
    fn brace_depth_after(line: &str, depth: usize) -> usize {
        let mut depth = depth;
        let mut characters = line.chars().peekable();
        let mut quote: Option<char> = None;
        while let Some(character) = characters.next() {
            match (quote, character) {
                (Some(_), '\\') => {
                    characters.next();
                }
                (Some(open), current) if current == open => quote = None,
                (Some(_), _) => {}
                (None, '"' | '\'') => quote = Some(character),
                (None, '{') => depth += 1,
                (None, '}') => depth = depth.saturating_sub(1),
                (None, _) => {}
            }
        }
        depth
    }

    fn is_type_declaration(line: &str) -> bool {
        let line = line.trim_start();
        if line.is_empty() {
            return false;
        }
        if line.starts_with('[') && line.trim_end().ends_with(']') {
            return true;
        }
        let mut rest = line;
        loop {
            if TYPE_KEYWORDS.iter().any(|keyword| rest.starts_with(keyword)) {
                return true;
            }
            match DECLARATION_MODIFIERS
                .iter()
                .find_map(|modifier| rest.strip_prefix(modifier))
            {
                Some(stripped) => rest = stripped,
                None => return false,
            }
        }
    }

    fn wrap_if_fragment(code: &str) -> String {
        let trimmed = code.trim();
        let only_comments = !trimmed.is_empty()
            && trimmed
                .lines()
                .all(|line| line.trim().is_empty() || line.trim().starts_with("//"));
        if only_comments {
            return format!("{trimmed}\n// snippet placeholder\nreturn;\n");
        }
        code.to_string()
    }

    fn target_framework(manifest: &std::path::Path) -> Result<String> {
        let source = std::fs::read_to_string(manifest)?;
        for element in ["TargetFramework", "TargetFrameworks"] {
            let opening = format!("<{element}>");
            let closing = format!("</{element}>");
            if let Some(start) = source.find(&opening)
                && let Some(end) = source[start + opening.len()..].find(&closing)
            {
                let value = &source[start + opening.len()..start + opening.len() + end];
                if let Some(framework) = value.split(';').map(str::trim).find(|value| !value.is_empty()) {
                    return Ok(framework.to_owned());
                }
            }
        }
        Err(crate::snippets::error::Error::Other(format!(
            "no target framework in {}",
            manifest.display()
        )))
    }

    fn configure_dotnet_environment(command: &mut std::process::Command, directory: &std::path::Path) -> Result<()> {
        for (name, path) in [
            ("DOTNET_CLI_HOME", directory.join(".dotnet")),
            ("NUGET_PACKAGES", directory.join(".nuget/packages")),
        ] {
            std::fs::create_dir_all(&path).map_err(|error| {
                crate::snippets::error::Error::Other(format!(
                    "creating isolated .NET directory {}: {error}",
                    path.display()
                ))
            })?;
            command.env(name, path);
        }
        Ok(())
    }

    /// ~keep `CS0246` (type or namespace not found) and `CS0234` (namespace has no such member)
    /// can only mean the referenced package is absent or stale. `CS0103` ("the name 'x' does not
    /// exist in the current context") is C#'s ambiguous unresolved-name diagnostic — the direct
    /// analogue of `TS2304`, rejected by task #130 — and `CS5001` (no entry point) is a defect in
    /// the emitted program, not a missing package. Accepting either made
    /// `runner::finalize_result` relabel a codegen defect as `Unavailable`.
    ///
    /// Line-scoped and all-must-match, like `typescript::is_dependency_error`, so a batch mixing
    /// one unresolved namespace with a genuine error is not relabeled wholesale.
    fn is_dependency_error_text(output: &str) -> bool {
        all_error_lines_match(
            output,
            |line| line.contains(": error CS"),
            |line| line.contains("CS0246") || line.contains("CS0234"),
        )
    }
}

impl SnippetValidator for CsharpValidator {
    fn language(&self) -> Language {
        Language::Csharp
    }

    fn is_available(&self) -> bool {
        which::which("dotnet").is_ok()
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

    fn validate_batch_in_session(
        &self,
        snippets: &[&Snippet],
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Option<Result<BatchValidation>> {
        Self::validate_batch_with_context(snippets, level, timeout_secs, session)
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

    fn is_dependency_error(&self, output: &str) -> bool {
        Self::is_dependency_error_text(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snippets::types::{SnippetMetadata, SourceOrigin};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    /// Whether `dotnet` runs, not merely resolves: a version-manager shim spawns fine then exits
    /// non-zero, so a PATH-only check leaves the skip below unreachable and fires the assert
    /// everywhere the .NET SDK is absent. ~keep
    fn dotnet_is_runnable() -> bool {
        static RUNNABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *RUNNABLE.get_or_init(|| {
            std::process::Command::new("dotnet")
                .arg("--version")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
        })
    }

    const TOOLCHAIN_TEST_TIMEOUT_SECS: u64 = 120;

    #[test]
    fn session_manifest_adds_a_real_project_reference() {
        if !dotnet_is_runnable() {
            return;
        }
        let root = tempfile::tempdir().expect("temporary root");
        let project = root.path().join("LocalFixture");
        let working = root.path().join("working");
        std::fs::create_dir_all(&project).expect("project directory");
        std::fs::create_dir_all(&working).expect("working directory");
        std::fs::write(
            project.join("LocalFixture.csproj"),
            "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><TargetFramework>net8.0</TargetFramework></PropertyGroup></Project>",
        )
        .expect("project manifest");
        std::fs::write(
            project.join("Value.cs"),
            "namespace LocalFixture; public static class Values { public const int Value = 1; }",
        )
        .expect("project source");
        let session = ValidationSession {
            language: Language::Csharp,
            working_directory: working,
            manifest: Some(project.join("LocalFixture.csproj")),
            fingerprint: "fixture".into(),
            env: BTreeMap::from([("DOTNET_CLI_TELEMETRY_OPTOUT".into(), "1".into())]),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        };

        let (status, output) = CsharpValidator::validate_with_context(
            &snippet("using LocalFixture; System.Console.WriteLine(Values.Value);"),
            ValidationLevel::TypeCheck,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            Some(&session),
        )
        .expect("validation runs");
        assert_eq!(status, SnippetStatus::Pass, "{output:?}");
    }

    #[test]
    fn derives_target_framework_from_referenced_project() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let manifest = directory.path().join("Fixture.csproj");
        std::fs::write(
            &manifest,
            "<Project><PropertyGroup><TargetFrameworks>net10.0;net9.0</TargetFrameworks></PropertyGroup></Project>",
        )
        .expect("project manifest");

        assert_eq!(CsharpValidator::target_framework(&manifest).unwrap(), "net10.0");
    }

    /// Attribution is asserted without a toolchain so the mapping itself is pinned rather than the
    /// SDK's willingness to run: a batch that blames one snippet for another's error is the one
    /// failure mode that makes batching worse than the per-snippet path it replaces. The
    /// diagnostic is repeated because MSBuild prints it inline and again under `Build FAILED.` ~keep
    #[test]
    fn an_msbuild_diagnostic_is_attributed_once_to_the_snippet_that_owns_its_file() {
        let units = [
            batch_unit("Snippet_first.cs"),
            batch_unit("Snippet_second.cs"),
            batch_unit("Snippet_third.cs"),
        ];
        let line = "/batch/Snippet_second.cs(7,26): error CS0103: The name 'Bogus' does not exist in the current context [/batch/Snippet.csproj]";
        let output = format!("{line}\n\nBuild FAILED.\n\n{line}\n    0 Warning(s)\n    1 Error(s)\n");

        let results = CsharpValidator::batch_results(&units, false, false, &output);

        assert_eq!(results.len(), 3);
        assert_eq!(results[0], (SnippetStatus::Pass, None));
        assert_eq!(results[2], (SnippetStatus::Pass, None));
        assert_eq!(results[1], (SnippetStatus::Fail, Some(line.to_owned())));
    }

    /// A build that failed with nothing attributable must fail every snippet carrying the real
    /// output — never silently pass the batch. ~keep
    #[test]
    fn an_unattributable_build_failure_fails_every_snippet_with_the_real_output() {
        let units = [batch_unit("Snippet_first.cs"), batch_unit("Snippet_second.cs")];
        let output = "/batch/Snippet.csproj : error NU1101: Unable to find package Fixture.\n";

        let results = CsharpValidator::batch_results(&units, false, false, output);

        assert_eq!(results.len(), 2);
        for result in &results {
            assert_eq!(result.0, SnippetStatus::Fail);
            assert_eq!(result.1.as_deref(), Some(output.trim()));
        }
    }

    /// `-warnaserror` is only passed at `TypeCheck`, so a warning must fail its snippet there and
    /// be ignored at `Compile` — exactly what the per-snippet path does with the same flags. ~keep
    #[test]
    fn a_warning_fails_its_snippet_only_when_warnaserror_is_in_effect() {
        let units = [batch_unit("Snippet_first.cs"), batch_unit("Snippet_second.cs")];
        let output = "/batch/Snippet_second.cs(9,13): warning CS0219: The variable 'unused' is assigned but never used [/batch/Snippet.csproj]\n";

        let type_check = CsharpValidator::batch_results(&units, true, false, output);
        let compile = CsharpValidator::batch_results(&units, false, true, output);

        assert_eq!(type_check[0].0, SnippetStatus::Pass);
        assert_eq!(type_check[1].0, SnippetStatus::Fail);
        assert_eq!(compile[0].0, SnippetStatus::Pass);
        assert_eq!(compile[1].0, SnippetStatus::Pass);
    }

    #[test]
    fn run_level_declines_batching() {
        let first = snippet("System.Console.WriteLine(1);");
        let second = snippet("System.Console.WriteLine(2);");

        let declined = CsharpValidator::validate_batch_with_context(&[&first, &second], ValidationLevel::Run, 5, None);

        assert!(declined.is_none());
    }

    #[test]
    fn a_batch_is_declined_when_two_snippets_share_a_declared_namespace() {
        let first = snippet("namespace Shared.Fixture;\npublic static class Alpha { }");
        let second = snippet("namespace Shared.Fixture;\npublic static class Beta { }");

        let declined =
            CsharpValidator::validate_batch_with_context(&[&first, &second], ValidationLevel::Compile, 5, None);

        assert!(declined.is_none());
    }

    /// Top-level statements are legal in exactly one compilation unit per project, so batching them
    /// as written is impossible; each snippet's statements are lifted into its own namespaced
    /// wrapper instead, with its `using` directives kept above namespace scope where C# requires
    /// them. ~keep
    #[test]
    fn top_level_statements_are_lifted_into_a_namespaced_wrapper_class() {
        let source = CsharpValidator::batch_source(
            "using LocalFixture;\nSystem.Console.WriteLine(Values.Value);",
            "AlefSnippetX",
        );

        assert_eq!(
            source,
            "using LocalFixture;\n\nnamespace AlefSnippetX;\n\ninternal static class Snippet\n{\n    internal static void Run(string[] args)\n    {\nSystem.Console.WriteLine(Values.Value);\n    }\n}\n"
        );
    }

    /// A type cannot be declared inside a method body, so trailing type declarations have to stay
    /// at namespace scope while the statements before them move into the wrapper. ~keep
    /// The defect: `is_type_declaration` rejected any indented line, and alef's own C# snippet
    /// generator indents a trailing `sealed class` by four spaces. The declaration was therefore
    /// left inside the wrapper method, where C# does not permit one, and 54 of one consumer's 283
    /// snippets failed on `CS1513: } expected`. ~keep
    #[test]
    fn an_indented_trailing_type_declaration_still_reaches_namespace_scope() {
        let source = CsharpValidator::batch_source(
            "System.Console.WriteLine(1);\n    sealed class Helper\n    {\n        public int Value => 1;\n    }\n",
            "AlefSnippetX",
        );

        assert!(
            source.contains("}\n\n    sealed class Helper"),
            "the class must sit outside the wrapper method: {source}"
        );
        assert!(
            !source.contains("Run(string[] args)\n    {\nSystem.Console.WriteLine(1);\n    sealed class"),
            "the class must not remain inside the method body: {source}"
        );
    }

    /// The control the column guard was actually reaching for: a `class` token inside a method body
    /// is not a trailing declaration, and splitting there would cut the statements in half.
    #[test]
    fn a_type_keyword_nested_inside_a_brace_block_is_not_a_split_point() {
        let (statements, declarations) = CsharpValidator::split_type_declarations(
            "if (true)\n{\n    var class_count = 1;\n    record Inner(int X);\n}\nSystem.Console.WriteLine(2);\n",
        );

        assert!(statements.contains("System.Console.WriteLine(2);"), "{statements}");
        assert!(
            declarations.is_empty(),
            "nothing here is a trailing declaration: {declarations}"
        );
    }

    #[test]
    fn trailing_type_declarations_stay_at_namespace_scope() {
        let source = CsharpValidator::batch_source(
            "System.Console.WriteLine(1);\n\npublic record Point(int X, int Y);",
            "AlefSnippetX",
        );

        assert_eq!(
            source,
            "namespace AlefSnippetX;\n\ninternal static class Snippet\n{\n    internal static void Run(string[] args)\n    {\nSystem.Console.WriteLine(1);\n    }\n}\n\npublic record Point(int X, int Y);\n"
        );
    }

    /// `await` is legal in top-level statements but not in a `void` method, so the wrapper's
    /// signature has to follow the snippet. The async form is only used when the snippet needs it,
    /// because an `async` method without an `await` is warning CS1998 — which `-warnaserror` at
    /// `TypeCheck` would turn into a failure for every snippet. ~keep
    #[test]
    fn an_awaiting_snippet_gets_an_async_wrapper_signature() {
        let awaiting = CsharpValidator::batch_source("await System.Threading.Tasks.Task.Yield();", "AlefSnippetX");
        let synchronous = CsharpValidator::batch_source("System.Console.WriteLine(1);", "AlefSnippetY");

        assert!(
            awaiting.contains("internal static async System.Threading.Tasks.Task RunAsync(string[] args)"),
            "{awaiting}"
        );
        assert!(
            synchronous.contains("internal static void Run(string[] args)"),
            "{synchronous}"
        );
    }

    #[test]
    fn a_batch_returns_exactly_one_result_per_snippet_in_input_order() {
        if !dotnet_is_runnable() {
            return;
        }
        let snippets = [
            snippet("System.Console.WriteLine(\"first\");"),
            snippet("System.Console.WriteLine(\"second\");"),
            snippet("System.Console.WriteLine(\"third\");"),
            snippet("System.Console.WriteLine(\"fourth\");"),
        ];
        let batch = snippets.iter().collect::<Vec<_>>();

        let results = CsharpValidator::validate_batch_with_context(
            &batch,
            ValidationLevel::Compile,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            None,
        )
        .expect("compile level batches")
        .expect("batch validation runs");

        assert_eq!(results.len(), 4);
        for (index, result) in results.iter().enumerate() {
            assert_eq!(result.0, SnippetStatus::Pass, "snippet {index}: {:?}", result.1);
        }
    }

    #[test]
    fn a_batch_fails_only_the_broken_snippet() {
        if !dotnet_is_runnable() {
            return;
        }
        let snippets = [
            snippet("System.Console.WriteLine(\"first\");"),
            snippet("System.Console.WriteLine(BogusType.Value);"),
            snippet("System.Console.WriteLine(\"third\");"),
        ];
        let batch = snippets.iter().collect::<Vec<_>>();

        let results = CsharpValidator::validate_batch_with_context(
            &batch,
            ValidationLevel::Compile,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            None,
        )
        .expect("compile level batches")
        .expect("batch validation runs");

        assert_eq!(results.len(), 3);
        assert_eq!(results[0], (SnippetStatus::Pass, None));
        assert_eq!(results[2], (SnippetStatus::Pass, None));
        assert_eq!(results[1].0, SnippetStatus::Fail);
        assert!(
            results[1]
                .1
                .as_deref()
                .is_some_and(|message| message.contains("BogusType")),
            "the failure must name the broken snippet's own symbol: {:?}",
            results[1].1
        );
    }

    fn batch_unit(file_name: &str) -> CsharpBatchUnit {
        CsharpBatchUnit {
            file_name: file_name.to_owned(),
            source: String::new(),
        }
    }

    fn snippet(code: &str) -> Snippet {
        Snippet {
            id: None,
            path: PathBuf::from("snippet.cs"),
            language: Language::Csharp,
            title: None,
            code: code.into(),
            start_line: 1,
            block_index: 0,
            annotation: None,
            metadata: SnippetMetadata::default(),
            source_origin: SourceOrigin {
                path: PathBuf::from("snippet.cs"),
                line: 1,
                block_index: 0,
            },
        }
    }
}
