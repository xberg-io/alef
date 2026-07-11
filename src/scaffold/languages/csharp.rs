use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::ApiSurface;
use crate::core::version::to_dotnet_assembly_version;
use crate::scaffold::naming::csharp_package_id;
use crate::{scaffold::scaffold_meta, scaffold::xml_escape};
use std::path::PathBuf;

/// Runtime Identifiers (RIDs) advertised by the binding NuGet package, each
/// paired with the Rust target triple that produces its native asset.
///
/// Every enabled platform must appear in `<RuntimeIdentifiers>` so the SDK packs
/// its `runtimes/<rid>/native/` payload into the NuGet artifact and consumers
/// resolve a matching native asset at restore time. Mismatches surface to
/// consumers as `warning CS8012: ... targets a different processor` followed by
/// `FileNotFoundException` at runtime — the exact failure mode observed on tslp
/// v1.9.0-rc.48. A RID whose triple is disabled via the workspace `[targets]`
/// opt-out table is dropped from the list.
pub(crate) const PUBLISHED_RUNTIME_IDENTIFIERS: &[(&str, &str)] = &[
    ("win-x64", "x86_64-pc-windows-msvc"),
    ("win-arm64", "aarch64-pc-windows-msvc"),
    ("linux-x64", "x86_64-unknown-linux-gnu"),
    ("linux-arm64", "aarch64-unknown-linux-gnu"),
    ("osx-x64", "x86_64-apple-darwin"),
    ("osx-arm64", "aarch64-apple-darwin"),
];

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
    let repository_csproj = meta
        .configured_repository
        .as_deref()
        .map(|repository| format!("    <RepositoryUrl>{}</RepositoryUrl>\n", xml_escape(repository)))
        .unwrap_or_default();

    let assembly_version = to_dotnet_assembly_version(version);
    let runtime_identifiers = PUBLISHED_RUNTIME_IDENTIFIERS
        .iter()
        .filter(|(_, triple)| config.target_enabled(triple))
        .map(|(rid, _)| *rid)
        .collect::<Vec<_>>()
        .join(";");

    format!(
        r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <TargetFramework>{target_framework}</TargetFramework>
    <RootNamespace>{namespace}</RootNamespace>
    <PackageId>{package_id}</PackageId>
    <Version>{version}</Version>
    <AssemblyVersion>{assembly_version}</AssemblyVersion>
    <FileVersion>{assembly_version}</FileVersion>
    <InformationalVersion>{version}</InformationalVersion>
    <Description>{description}</Description>
    <PackageLicenseFile>LICENSE</PackageLicenseFile>
{repository}{authors}    <Company>Alef Team</Company>
    <Product>{namespace}</Product>
    <AllowUnsafeBlocks>true</AllowUnsafeBlocks>
    <Nullable>enable</Nullable>
    <!-- AnyCPU managed assembly so the PE `Machine` header stays processor-neutral
         and consumers never see `warning CS8012: ... targets a different processor`.
         Native asset resolution at consumer build time is driven by the
         `runtimes/<rid>/native/` payload baked into the NuGet package — not a
         single `<RuntimeIdentifier>` on the package author's side, which would
         force runtime-specific output and break AnyCPU packaging. -->
    <PlatformTarget>AnyCPU</PlatformTarget>
    <RuntimeIdentifiers>{runtime_identifiers}</RuntimeIdentifiers>
  </PropertyGroup>

  <ItemGroup>
    <None Include="../../../LICENSE" Pack="true" PackagePath="/" />
    <None Include="runtimes/**" Pack="true" PackagePath="runtimes/" CopyToOutputDirectory="PreserveNewest" />
  </ItemGroup>

  <ItemGroup>
    <Compile Include="../src/**/*.cs" />
  </ItemGroup>
{capsule_package_refs}</Project>
"#,
        target_framework = target_framework,
        namespace = namespace,
        package_id = package_id,
        version = version,
        assembly_version = assembly_version,
        runtime_identifiers = runtime_identifiers,
        description = meta.description,
        repository = repository_csproj,
        authors = authors_csproj,
        capsule_package_refs = capsule_package_refs(config),
    )
}

/// Render a `<PackageReference>` ItemGroup for host-native capsule (Language) passthrough
/// dependencies (e.g. NuGet `TreeSitter.DotNet`). Empty when no capsule types are configured.
fn capsule_package_refs(config: &ResolvedCrateConfig) -> String {
    let mut deps: Vec<(String, String)> = config
        .csharp
        .as_ref()
        .map(|c| {
            c.capsule_types
                .values()
                .filter(|cap| !cap.package.is_empty())
                .map(|cap| (cap.package.clone(), cap.package_version.clone()))
                .collect()
        })
        .unwrap_or_default();
    deps.sort();
    deps.dedup();
    if deps.is_empty() {
        return String::new();
    }
    let refs: String = deps
        .iter()
        .map(|(pkg, ver)| format!("    <PackageReference Include=\"{pkg}\" Version=\"{ver}\" />\n"))
        .collect();
    format!("\n  <ItemGroup>\n{refs}  </ItemGroup>\n")
}

pub(crate) fn scaffold_csharp(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let namespace = config.csharp_namespace();
    let content = render_csharp_csproj(config, &api.version);

    Ok(vec![
        GeneratedFile {
            // alef-publish stages the FFI shared libraries.  `../../../LICENSE` from that
            path: PathBuf::from(format!("packages/csharp/{0}/{0}.csproj", namespace)),
            content,
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/csharp/.editorconfig"),
            content: "root = true\n\n[*.cs]\nindent_style = space\nindent_size = 4\nmax_line_length = 120\nend_of_line = lf\ncharset = utf-8\ntrim_trailing_whitespace = true\ninsert_final_newline = true\n".to_string(),
            generated_header: false,
        },
    ])
}
