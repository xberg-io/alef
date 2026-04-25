use crate::{scaffold_meta, xml_escape};
use alef_core::backend::GeneratedFile;
use alef_core::config::AlefConfig;
use alef_core::ir::ApiSurface;
use std::path::PathBuf;

pub(crate) fn scaffold_csharp(api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let namespace = config.csharp_namespace();
    let version = &api.version;

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

    let content = format!(
        r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <TargetFramework>{target_framework}</TargetFramework>
    <RootNamespace>{namespace}</RootNamespace>
    <PackageId>{namespace}</PackageId>
    <Version>{version}</Version>
    <Description>{description}</Description>
    <PackageLicenseFile>LICENSE</PackageLicenseFile>
    <RepositoryUrl>{repository}</RepositoryUrl>
{authors}    <AllowUnsafeBlocks>true</AllowUnsafeBlocks>
  </PropertyGroup>

  <ItemGroup>
    <None Include="../../../LICENSE" Pack="true" PackagePath="/" />
    <None Include="runtimes/**" Pack="true" PackagePath="runtimes/" CopyToOutputDirectory="PreserveNewest" />
  </ItemGroup>
</Project>
"#,
        target_framework = target_framework,
        namespace = namespace,
        version = version,
        description = meta.description,
        repository = meta.repository,
        authors = authors_csproj,
    );

    Ok(vec![
        GeneratedFile {
            path: PathBuf::from(format!("packages/csharp/{}.csproj", namespace)),
            content,
            generated_header: true,
        },
        GeneratedFile {
            path: PathBuf::from("packages/csharp/.editorconfig"),
            content: "root = true\n\n[*.cs]\nindent_style = space\nindent_size = 4\nmax_line_length = 120\nend_of_line = lf\ncharset = utf-8\ntrim_trailing_whitespace = true\ninsert_final_newline = true\n".to_string(),
            generated_header: false,
        },
    ])
}
