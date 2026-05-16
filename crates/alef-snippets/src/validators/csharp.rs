use crate::error::Result;
use crate::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::validators::{SnippetValidator, run_command};
use tempfile::TempDir;

pub struct CsharpValidator;

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
        let dir = TempDir::new()?;
        let project_path = dir.path().join("Snippet.csproj");
        let project = r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <OutputType>Exe</OutputType>
    <TargetFramework>net8.0</TargetFramework>
    <Nullable>enable</Nullable>
    <ImplicitUsings>enable</ImplicitUsings>
  </PropertyGroup>
</Project>
"#;
        std::fs::write(&project_path, project)?;
        std::fs::write(dir.path().join("Program.cs"), snippet.code.trim())?;

        let mut command = std::process::Command::new("dotnet");
        match level {
            ValidationLevel::Syntax | ValidationLevel::Compile => {
                command.args(["build", "--nologo", "-v", "quiet"]).current_dir(dir.path());
            }
            ValidationLevel::Run => {
                command.args(["run", "--nologo"]).current_dir(dir.path());
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
        ValidationLevel::Run
    }

    fn is_dependency_error(&self, output: &str) -> bool {
        output.contains("CS0246") || output.contains("CS0234") || output.contains("CS0103")
    }
}
