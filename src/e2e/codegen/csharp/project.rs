//! C# e2e project and shared test setup rendering.

use crate::core::hash::{self, CommentStyle};
use crate::core::template_versions as tv;
use std::fmt::Write as FmtWrite;

pub(super) fn render_csproj(
    pkg_name: &str,
    pkg_path: &str,
    pkg_version: &str,
    dep_mode: crate::e2e::config::DependencyMode,
) -> String {
    let pkg_ref = match dep_mode {
        crate::e2e::config::DependencyMode::Registry => {
            format!("    <PackageReference Include=\"{pkg_name}\" Version=\"{pkg_version}\" />")
        }
        crate::e2e::config::DependencyMode::Local => {
            format!("    <ProjectReference Include=\"{pkg_path}\" />")
        }
    };
    crate::e2e::template_env::render(
        "csharp/csproj.jinja",
        minijinja::context! {
            pkg_ref => pkg_ref,
            microsoft_net_test_sdk_version => tv::nuget::MICROSOFT_NET_TEST_SDK,
            xunit_version => tv::nuget::XUNIT,
            xunit_runner_version => tv::nuget::XUNIT_RUNNER_VISUALSTUDIO,
        },
    )
}

pub(super) fn render_test_setup(needs_mock_server: bool, test_documents_dir: &str, namespace: &str) -> String {
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
    out.push_str("    [ModuleInitializer]\n");
    out.push_str("    internal static void Init()\n");
    out.push_str("    {\n");
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
