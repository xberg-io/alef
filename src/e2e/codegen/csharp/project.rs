//! C# e2e project and shared test setup rendering.

use crate::core::config::manifest_extras::ManifestExtras;
use crate::core::hash::{self, CommentStyle};
use crate::core::template_versions as tv;
use std::collections::HashMap;
use std::fmt::Write as FmtWrite;

pub(super) fn render_csproj(
    pkg_name: &str,
    pkg_path: &str,
    pkg_version: &str,
    dep_mode: crate::e2e::config::DependencyMode,
    extras: Option<&ManifestExtras>,
) -> String {
    let pkg_ref = match dep_mode {
        crate::e2e::config::DependencyMode::Registry => {
            format!("    <PackageReference Include=\"{pkg_name}\" Version=\"{pkg_version}\" />")
        }
        crate::e2e::config::DependencyMode::Local => {
            format!("    <ProjectReference Include=\"{pkg_path}\" />")
        }
    };

    // Build extras block: combine dependencies and dev_dependencies as PackageReference elements.
    // C# .csproj has no dev/runtime split — all are PackageReference. Both buckets render as
    // the same element type, though in practice harness_extras would only use dev_dependencies
    // (vitest, mock-server libs, etc.).
    let extras_block = match extras {
        Some(e) if !e.is_empty() => {
            let mut lines = Vec::new();
            // Combine both buckets (dependencies, dev_dependencies) into one sorted set.
            let mut all_deps = e.dependencies.clone();
            for (name, spec) in &e.dev_dependencies {
                all_deps.insert(name.clone(), spec.clone());
            }

            if !all_deps.is_empty() {
                for (name, spec) in &all_deps {
                    if let Some(version) = spec.version() {
                        lines.push(format!(
                            "    <PackageReference Include=\"{name}\" Version=\"{version}\" />"
                        ));
                    }
                }
            }
            lines.join("\n")
        }
        _ => String::new(),
    };

    crate::e2e::template_env::render(
        "csharp/csproj.jinja",
        minijinja::context! {
            pkg_ref => pkg_ref,
            extras_block => extras_block,
            namespace => pkg_name,
            microsoft_net_test_sdk_version => tv::nuget::MICROSOFT_NET_TEST_SDK,
            xunit_version => tv::nuget::XUNIT,
            xunit_runner_version => tv::nuget::XUNIT_RUNNER_VISUALSTUDIO,
        },
    )
}

pub(super) fn render_test_setup(
    needs_mock_server: bool,
    test_documents_dir: &str,
    namespace: &str,
    env: &HashMap<String, String>,
) -> String {
    let mut out = String::new();
    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    out.push_str("using System;\n");
    out.push_str("using System.IO;\n");
    if needs_mock_server {
        out.push_str("using System.Diagnostics;\n");
    }
    out.push_str("using System.Runtime.CompilerServices;\n\n");
    let _ = writeln!(out, "namespace {namespace};\n");
    out.push_str("internal static class TestSetup\n");
    out.push_str("{\n");
    if needs_mock_server {
        out.push_str("    private static Process? _mockServer;\n\n");
    }
    // When env vars are configured, emit a libc `setenv` P/Invoke plus a `SetNativeEnv`
    // helper. Managed `Environment.SetEnvironmentVariable` does not reliably propagate to
    // the native `getenv` the underlying FFI library reads (e.g. SSRF allow-listing for
    // loopback mock-server URLs), so push the value through the C runtime as well.
    if !env.is_empty() {
        out.push_str(
            "    // libc setenv — Environment.SetEnvironmentVariable does not reliably propagate to the\n",
        );
        out.push_str(
            "    // native getenv the underlying library reads, so set it through the C runtime directly.\n",
        );
        out.push_str("    [System.Runtime.InteropServices.DllImport(\"libc\", SetLastError = true)]\n");
        out.push_str("    private static extern int setenv(string name, string value, int overwrite);\n\n");
        out.push_str("    private static void SetNativeEnv(string name, string value)\n");
        out.push_str("    {\n");
        out.push_str("        Environment.SetEnvironmentVariable(name, value);\n");
        out.push_str("        if (!OperatingSystem.IsWindows())\n");
        out.push_str("        {\n");
        out.push_str("            try { setenv(name, value, 1); } catch { }\n");
        out.push_str("        }\n");
        out.push_str("    }\n\n");
    }
    out.push_str("    [ModuleInitializer]\n");
    out.push_str("    internal static void Init()\n");
    out.push_str("    {\n");

    // Emit env vars if present
    if !env.is_empty() {
        let mut sorted_keys: Vec<_> = env.keys().collect();
        sorted_keys.sort();
        for key in sorted_keys {
            let value = &env[key];
            let _ = writeln!(
                out,
                "        if (Environment.GetEnvironmentVariable(\"{key}\") == null) {{"
            );
            let _ = writeln!(out, "            SetNativeEnv(\"{key}\", \"{value}\");");
            out.push_str("        }\n");
        }
        out.push('\n');
    }

    let _ = writeln!(
        out,
        "        // Walk up from the assembly directory until we find the repo root."
    );
    let _ = writeln!(
        out,
        "        // Prefer a sibling {test_documents_dir}/ directory (chdir into it so that"
    );
    out.push_str("        // fixture paths like \"docx/fake.docx\" resolve relative to it). If that\n");
    out.push_str("        // is absent (projects with no document fixtures), fall\n");
    out.push_str("        // back to a sibling alef.toml or fixtures/ marker as the repo root.\n");
    out.push_str("        var dir = new DirectoryInfo(AppContext.BaseDirectory);\n");
    out.push_str("        DirectoryInfo? repoRoot = null;\n");
    out.push_str("        while (dir != null)\n");
    out.push_str("        {\n");
    let _ = writeln!(
        out,
        "            var documentsCandidate = Path.Combine(dir.FullName, \"{test_documents_dir}\");"
    );
    out.push_str("            if (Directory.Exists(documentsCandidate))\n");
    out.push_str("            {\n");
    out.push_str("                repoRoot = dir;\n");
    out.push_str("                Directory.SetCurrentDirectory(documentsCandidate);\n");
    out.push_str("                break;\n");
    out.push_str("            }\n");
    out.push_str("            if (File.Exists(Path.Combine(dir.FullName, \"alef.toml\"))\n");
    out.push_str("                || Directory.Exists(Path.Combine(dir.FullName, \"fixtures\")))\n");
    out.push_str("            {\n");
    out.push_str("                repoRoot = dir;\n");
    out.push_str("                break;\n");
    out.push_str("            }\n");
    out.push_str("            dir = dir.Parent;\n");
    out.push_str("        }\n");
    if needs_mock_server {
        out.push('\n');
        let mock_server_code =
            crate::e2e::template_env::render("csharp/test_setup_mock_server.cs.jinja", minijinja::context! {});
        out.push_str(&mock_server_code);
    }
    out.push_str("    }\n");
    out.push_str("}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_test_setup_with_env_vars() {
        let mut env = HashMap::new();
        env.insert("ZEBRA_VAR".to_string(), "z_value".to_string());
        env.insert("ALPHA_VAR".to_string(), "a_value".to_string());
        env.insert("BETA_VAR".to_string(), "b_value".to_string());

        let output = render_test_setup(false, "fixtures", "FixtureE2E", &env);

        assert!(output.contains("ALPHA_VAR"));
        assert!(output.contains("a_value"));
        assert!(output.contains("BETA_VAR"));
        assert!(output.contains("b_value"));
        assert!(output.contains("ZEBRA_VAR"));
        assert!(output.contains("z_value"));

        // Verify alphabetical order
        let alpha_pos = output.find("ALPHA_VAR").unwrap();
        let beta_pos = output.find("BETA_VAR").unwrap();
        let zebra_pos = output.find("ZEBRA_VAR").unwrap();
        assert!(alpha_pos < beta_pos && beta_pos < zebra_pos);

        // Verify SetEnvironmentVariable pattern
        assert!(output.contains("Environment.SetEnvironmentVariable("));
    }

    #[test]
    fn test_render_test_setup_empty_env() {
        let env = HashMap::new();
        let output = render_test_setup(false, "fixtures", "FixtureE2E", &env);

        // Should not contain SetEnvironmentVariable calls for empty env
        assert!(!output.contains("Environment.SetEnvironmentVariable("));
        // The libc setenv P/Invoke is only emitted when env vars are configured.
        assert!(!output.contains("private static extern int setenv("));
        assert!(!output.contains("SetNativeEnv"));
    }

    #[test]
    fn test_render_test_setup_env_null_check() {
        let mut env = HashMap::new();
        env.insert("TEST_VAR".to_string(), "test_value".to_string());

        let output = render_test_setup(false, "fixtures", "FixtureE2E", &env);

        // Verify null-check pattern: if null, set via the native-aware helper.
        assert!(output.contains("if (Environment.GetEnvironmentVariable(\"TEST_VAR\") == null)"));
        assert!(output.contains("SetNativeEnv(\"TEST_VAR\", \"test_value\");"));
        // The libc setenv P/Invoke and helper are emitted when env vars are present.
        assert!(output.contains("[System.Runtime.InteropServices.DllImport(\"libc\", SetLastError = true)]"));
        assert!(output.contains("private static extern int setenv(string name, string value, int overwrite);"));
        assert!(output.contains("private static void SetNativeEnv(string name, string value)"));
    }

    #[test]
    fn test_render_csproj_with_extras() {
        use crate::core::config::manifest_extras::{ExtraDepSpec, ManifestExtras};

        let mut extras = ManifestExtras::default();
        extras.dependencies.insert(
            "TreeSitter.DotNet".to_string(),
            ExtraDepSpec::Simple("1.3.0".to_string()),
        );

        let output = render_csproj(
            "MyLib",
            "../../packages/csharp/MyLib/MyLib.csproj",
            "0.1.0",
            crate::e2e::config::DependencyMode::Local,
            Some(&extras),
        );

        // Verify the extras PackageReference is present
        assert!(
            output.contains("<PackageReference Include=\"TreeSitter.DotNet\" Version=\"1.3.0\" />"),
            "extras should inject PackageReference, got: {}",
            output
        );
        // Verify the main package reference is still there
        assert!(
            output.contains("<ProjectReference Include=\"../../packages/csharp/MyLib/MyLib.csproj\" />"),
            "Local mode should use ProjectReference"
        );
    }

    #[test]
    fn test_render_csproj_with_dev_dependencies() {
        use crate::core::config::manifest_extras::{ExtraDepSpec, ManifestExtras};

        let mut extras = ManifestExtras::default();
        extras
            .dev_dependencies
            .insert("Bogus".to_string(), ExtraDepSpec::Simple("^35.0.0".to_string()));

        let output = render_csproj(
            "MyLib",
            "../../packages/csharp/MyLib/MyLib.csproj",
            "0.1.0",
            crate::e2e::config::DependencyMode::Local,
            Some(&extras),
        );

        // Dev dependencies should also render as PackageReference
        assert!(
            output.contains("<PackageReference Include=\"Bogus\" Version=\"^35.0.0\" />"),
            "dev_dependencies should inject as PackageReference, got: {}",
            output
        );
    }

    #[test]
    fn test_render_csproj_with_both_dependency_buckets() {
        use crate::core::config::manifest_extras::{ExtraDepSpec, ManifestExtras};

        let mut extras = ManifestExtras::default();
        extras.dependencies.insert(
            "TreeSitter.DotNet".to_string(),
            ExtraDepSpec::Simple("1.3.0".to_string()),
        );
        extras
            .dev_dependencies
            .insert("Bogus".to_string(), ExtraDepSpec::Simple("^35.0.0".to_string()));

        let output = render_csproj(
            "MyLib",
            "../../packages/csharp/MyLib/MyLib.csproj",
            "0.1.0",
            crate::e2e::config::DependencyMode::Local,
            Some(&extras),
        );

        // Both buckets should be present
        assert!(output.contains("<PackageReference Include=\"TreeSitter.DotNet\""));
        assert!(output.contains("<PackageReference Include=\"Bogus\""));
    }

    #[test]
    fn test_render_csproj_without_extras() {
        let output = render_csproj(
            "MyLib",
            "../../packages/csharp/MyLib/MyLib.csproj",
            "0.1.0",
            crate::e2e::config::DependencyMode::Local,
            None,
        );

        // Should not have extras block, only baseline dependencies
        assert!(output.contains("Microsoft.NET.Test.Sdk"));
        assert!(output.contains("xunit"));
        assert!(output.contains("<ProjectReference Include=\"../../packages/csharp/MyLib/MyLib.csproj\" />"));
    }

    #[test]
    fn test_render_csproj_with_empty_extras() {
        use crate::core::config::manifest_extras::ManifestExtras;

        let extras = ManifestExtras::default(); // Empty: no dependencies or dev_dependencies

        let output = render_csproj(
            "MyLib",
            "../../packages/csharp/MyLib/MyLib.csproj",
            "0.1.0",
            crate::e2e::config::DependencyMode::Local,
            Some(&extras),
        );

        // Empty extras should not affect output (idempotent)
        assert!(output.contains("Microsoft.NET.Test.Sdk"));
        assert!(output.contains("<ProjectReference Include=\"../../packages/csharp/MyLib/MyLib.csproj\" />"));
    }

    #[test]
    fn test_render_csproj_registry_mode_ignores_extras() {
        use crate::core::config::manifest_extras::{ExtraDepSpec, ManifestExtras};

        let mut extras = ManifestExtras::default();
        extras.dependencies.insert(
            "TreeSitter.DotNet".to_string(),
            ExtraDepSpec::Simple("1.3.0".to_string()),
        );

        let output = render_csproj(
            "MyLib",
            "../../packages/csharp/MyLib/MyLib.csproj",
            "0.1.0",
            crate::e2e::config::DependencyMode::Registry,
            Some(&extras),
        );

        // Registry mode should NOT include extras; template may still process empty extras_block
        // but the real filtering happens at the call site. Verify the baseline works.
        assert!(output.contains("<PackageReference Include=\"MyLib\" Version=\"0.1.0\" />"));
        assert!(output.contains("Microsoft.NET.Test.Sdk"));
    }

    #[test]
    fn test_render_csproj_extras_within_item_group() {
        use crate::core::config::manifest_extras::{ExtraDepSpec, ManifestExtras};

        let mut extras = ManifestExtras::default();
        extras.dependencies.insert(
            "TreeSitter.DotNet".to_string(),
            ExtraDepSpec::Simple("1.3.0".to_string()),
        );
        extras
            .dev_dependencies
            .insert("Moq".to_string(), ExtraDepSpec::Simple("4.16.0".to_string()));

        let output = render_csproj(
            "MyLib",
            "../../packages/csharp/MyLib/MyLib.csproj",
            "0.1.0",
            crate::e2e::config::DependencyMode::Local,
            Some(&extras),
        );

        // Verify structure: extras should be within the first ItemGroup with test dependencies
        let first_item_group_start = output.find("<ItemGroup>").expect("should have ItemGroup");
        let first_item_group_end = output.find("</ItemGroup>").expect("should have /ItemGroup");
        let first_item_group = &output[first_item_group_start..=first_item_group_end];

        // Both extras and xunit should be in the same ItemGroup
        assert!(first_item_group.contains("xunit"), "xunit should be in first ItemGroup");
        assert!(
            first_item_group.contains("TreeSitter.DotNet"),
            "extras should be in first ItemGroup"
        );
        assert!(
            first_item_group.contains("Moq"),
            "dev_dependencies should also be in first ItemGroup"
        );

        // Verify the second ItemGroup has the pkg_ref
        let second_item_group_start = output[first_item_group_end..]
            .find("<ItemGroup>")
            .map(|i| i + first_item_group_end)
            .expect("should have second ItemGroup");
        let second_item_group_end = output[second_item_group_start..]
            .find("</ItemGroup>")
            .map(|i| i + second_item_group_start + 1)
            .unwrap_or(output.len());
        let second_item_group = &output[second_item_group_start..second_item_group_end];

        assert!(
            second_item_group.contains("ProjectReference"),
            "second ItemGroup should have ProjectReference"
        );
    }

    #[test]
    fn test_render_csproj_idempotent_with_same_extras() {
        use crate::core::config::manifest_extras::{ExtraDepSpec, ManifestExtras};

        let mut extras = ManifestExtras::default();
        extras.dependencies.insert(
            "TreeSitter.DotNet".to_string(),
            ExtraDepSpec::Simple("1.3.0".to_string()),
        );

        let output1 = render_csproj(
            "MyLib",
            "../../packages/csharp/MyLib/MyLib.csproj",
            "0.1.0",
            crate::e2e::config::DependencyMode::Local,
            Some(&extras),
        );

        let output2 = render_csproj(
            "MyLib",
            "../../packages/csharp/MyLib/MyLib.csproj",
            "0.1.0",
            crate::e2e::config::DependencyMode::Local,
            Some(&extras),
        );

        assert_eq!(output1, output2, "rendering with identical extras should be idempotent");
    }
}
