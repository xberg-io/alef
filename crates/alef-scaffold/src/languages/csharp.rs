use crate::naming::csharp_package_id;
use crate::{scaffold_meta, xml_escape};
use alef_core::backend::GeneratedFile;
use alef_core::config::ResolvedCrateConfig;
use alef_core::ir::ApiSurface;
use std::path::PathBuf;

/// Render just the `.csproj` XML content for the given config and version string.
///
/// The produced csproj is designed to live at
/// `packages/csharp/<Namespace>/<Namespace>.csproj`, where:
/// - `../../../LICENSE` reaches the workspace root (3 path components deep)
/// - `runtimes/**` matches `packages/csharp/<Namespace>/runtimes/` — the exact
///   directory where `alef-publish` stages the FFI shared libraries
///
/// This is exposed as a `pub` function so `alef-publish` can regenerate the
/// csproj before invoking `dotnet pack`, guaranteeing the glob paths are always
/// in sync with the staging layout regardless of what is committed on disk.
pub fn render_csharp_csproj(config: &ResolvedCrateConfig, version: &str) -> String {
    let meta = scaffold_meta(config);
    let namespace = config.csharp_namespace();
    let package_id = csharp_package_id(config);

    let target_framework = config
        .csharp
        .as_ref()
        .and_then(|c| c.target_framework.clone())
        .unwrap_or_else(|| "net10.0".to_string());

    let authors_csproj = if meta.authors.is_empty() {
        String::new()
    } else {
        let escaped: Vec<String> = meta.authors.iter().map(|a| xml_escape(a)).collect();
        format!("    <Authors>{}</Authors>\n", escaped.join(";"))
    };

    format!(
        r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <TargetFramework>{target_framework}</TargetFramework>
    <RootNamespace>{namespace}</RootNamespace>
    <PackageId>{package_id}</PackageId>
    <Version>{version}</Version>
    <Description>{description}</Description>
    <PackageLicenseFile>LICENSE</PackageLicenseFile>
    <RepositoryUrl>{repository}</RepositoryUrl>
{authors}    <AllowUnsafeBlocks>true</AllowUnsafeBlocks>
    <Nullable>enable</Nullable>
  </PropertyGroup>

  <ItemGroup>
    <None Include="../../../LICENSE" Pack="true" PackagePath="/" />
    <None Include="runtimes/**" Pack="true" PackagePath="runtimes/" CopyToOutputDirectory="PreserveNewest" />
  </ItemGroup>

  <ItemGroup>
    <Compile Include="../src/**/*.cs" />
  </ItemGroup>
</Project>
"#,
        target_framework = target_framework,
        namespace = namespace,
        package_id = package_id,
        version = version,
        description = meta.description,
        repository = meta.repository,
        authors = authors_csproj,
    )
}

pub(crate) fn scaffold_csharp(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let namespace = config.csharp_namespace();
    let content = render_csharp_csproj(config, &api.version);

    Ok(vec![
        GeneratedFile {
            // Place the csproj under packages/csharp/<Namespace>/<Namespace>.csproj so
            // the `runtimes/**` glob resolves to
            // packages/csharp/<Namespace>/runtimes/ — the exact directory where
            // alef-publish stages the FFI shared libraries.  `../../../LICENSE` from that
            // subdirectory (3 levels deep) reaches the workspace root.
            // alef-publish's find_csproj also looks here first, so no scanning fallback is needed.
            path: PathBuf::from(format!("packages/csharp/{0}/{0}.csproj", namespace)),
            content,
            // Scaffold-once so consumers can extend metadata (deps, runtime
            // configs, package metadata) without alef stomping on it.
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/csharp/.editorconfig"),
            content: "root = true\n\n[*.cs]\nindent_style = space\nindent_size = 4\nmax_line_length = 120\nend_of_line = lf\ncharset = utf-8\ntrim_trailing_whitespace = true\ninsert_final_newline = true\n".to_string(),
            generated_header: false,
        },
    ])
}
