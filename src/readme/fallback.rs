fn to_pascal_case(s: &str) -> String {
    s.to_upper_camel_case()
}

use crate::backends::gleam::naming::{gleam_app_name, gleam_nif_module};
use crate::core::backend::GeneratedFile;
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use heck::ToUpperCamelCase;
use std::path::PathBuf;

pub(super) fn generate_readme_hardcoded(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    lang: Language,
) -> anyhow::Result<GeneratedFile> {
    let name = &config.name;
    let description = config
        .scaffold
        .as_ref()
        .and_then(|s| s.description.clone())
        .unwrap_or_else(|| format!("Bindings for {}", name));
    let repository = config.github_repo();
    let example_pointer = format!("See {repository} for usage examples.");

    let (lang_display, install_instructions, example_code, dir_name) = match lang {
        // Examples are derived from the API surface so the snippet shows a real call
        // signature instead of a `// placeholder. Falls back to a "see main README"
        // pointer when the API has no public functions to demonstrate.
        Language::Python => {
            let module = config.python_module_name().trim_start_matches('_').to_string();
            let example_body = api
                .functions
                .first()
                .map(|f| {
                    format!(
                        "# result = {module}.{name}(...)\n# See the main repository's docs for full usage.",
                        name = f.name
                    )
                })
                .unwrap_or_else(|| format!("# {example_pointer}"));
            (
                "Python",
                format!("```bash\npip install {name}\n```"),
                format!("```python\nimport {module}\n\n{example_body}\n```"),
                "python",
            )
        }
        Language::Node => {
            let pkg = config.node_package_name();
            let example_body = api
                .functions
                .first()
                .map(|f| {
                    format!(
                        "// const result = await {fname}(...);\n// See the main repository's docs for full usage.",
                        fname = to_camel(&f.name)
                    )
                })
                .unwrap_or_else(|| format!("// {example_pointer}"));
            (
                "Node.js",
                format!("```bash\nnpm install {pkg}\n```"),
                format!("```typescript\nimport {{ /* ... */ }} from '{pkg}';\n\n{example_body}\n```"),
                "node",
            )
        }
        Language::Ruby => {
            let gem = config.ruby_gem_name();
            let example_body = format!("# {example_pointer}");
            (
                "Ruby",
                format!("```bash\ngem install {gem}\n```"),
                format!("```ruby\nrequire '{gem}'\n\n{example_body}\n```"),
                "ruby",
            )
        }
        Language::Php => {
            let ext = config.php_extension_name();
            let example_body = format!("// {example_pointer}");
            // Composer requires a `<vendor>/<package>` form. Derive the vendor
            // from the configured repository URL; fall back to the crate name
            // (which produces `<crate>/<crate>` — recognizably wrong without
            // smuggling a specific organization's name).
            let vendor = config
                .try_github_repo()
                .ok()
                .as_deref()
                .and_then(crate::core::config::derive_repo_org)
                .unwrap_or_else(|| name.clone());
            (
                "PHP",
                format!("```bash\ncomposer require {vendor}/{name}\n```"),
                format!("```php\n<?php\n\nuse {ext};\n\n{example_body}\n```"),
                "php",
            )
        }
        Language::Elixir => {
            let app = config.elixir_app_name();
            let module = capitalize_first(&app);
            let example_body = format!("# {example_pointer}");
            (
                "Elixir",
                format!(
                    "Add `:{app}` to your `mix.exs` dependencies:\n\n```elixir\ndefp deps do\n  [\n    {{:{app}, \"~> {version}\"}}\n  ]\nend\n```",
                    version = api.version,
                ),
                format!("```elixir\n{module}.hello()\n\n{example_body}\n```"),
                "elixir",
            )
        }
        Language::Go => {
            let module = config.go_module();
            let example_body = format!("\t// {example_pointer}");
            (
                "Go",
                format!("```bash\ngo get {module}\n```"),
                format!("```go\npackage main\n\nimport \"{module}\"\n\nfunc main() {{\n{example_body}\n}}\n```"),
                "go",
            )
        }
        Language::Java => {
            let package = config.java_package();
            let example_body = format!("// {example_pointer}");
            (
                "Java",
                format!(
                    "Add to your `pom.xml`:\n\n```xml\n<dependency>\n    <groupId>{package}</groupId>\n    <artifactId>{name}</artifactId>\n    <version>{version}</version>\n</dependency>\n```",
                    version = api.version,
                ),
                format!("```java\nimport {package}.*;\n\n{example_body}\n```"),
                "java",
            )
        }
        Language::Csharp => {
            let ns = config.csharp_namespace();
            let example_body = format!("// {example_pointer}");
            (
                "C#",
                format!("```bash\ndotnet add package {ns}\n```"),
                format!("```csharp\nusing {ns};\n\n{example_body}\n```"),
                "csharp",
            )
        }
        Language::Ffi => {
            let header = config.ffi_header_name();
            let example_body = format!("    // {example_pointer}");
            (
                "FFI (C/C++)",
                format!(
                    "Link against `lib{name}_ffi` and include `{header}`.\n\nSee the build instructions in the main repository.",
                ),
                format!("```c\n#include \"{header}\"\n\nint main(void) {{\n{example_body}\n    return 0;\n}}\n```"),
                "ffi",
            )
        }
        Language::Wasm => {
            let example_body = format!("// {example_pointer}");
            (
                "WebAssembly",
                format!("```bash\nnpm install {name}-wasm\n```"),
                format!("```javascript\nimport init from '{name}-wasm';\n\nawait init();\n{example_body}\n```"),
                "wasm",
            )
        }
        Language::R => {
            let pkg = config.r_package_name();
            let example_body = format!("# {example_pointer}");
            (
                "R",
                format!("```r\ninstall.packages('{pkg}')\n```"),
                format!("```r\nlibrary({pkg})\n\n{example_body}\n```"),
                "r",
            )
        }
        Language::Rust => {
            let import = config.core_import_name();
            let example_body = format!("// {example_pointer}");
            (
                "Rust",
                format!("```bash\ncargo add {name}\n```"),
                format!("```rust\nuse {import};\n\n{example_body}\n```"),
                "rust",
            )
        }
        Language::Kotlin => {
            let module = config.name.replace('-', "_");
            (
                "Kotlin",
                format!(
                    "Add the generated package to your `build.gradle.kts`:\n\n```kotlin\ndependencies {{\n    implementation(\"{}:{}:VERSION\")\n}}\n```",
                    config.kotlin_package(),
                    module
                ),
                format!(
                    "```kotlin\nimport {}.{}\n\n// Call generated APIs through the {} object.\n```",
                    config.kotlin_package(),
                    to_pascal_case(&config.name),
                    to_pascal_case(&config.name)
                ),
                "kotlin",
            )
        }
        Language::KotlinAndroid => {
            let module = config.name.replace('-', "_");
            (
                "Kotlin/Android",
                format!(
                    "Add the generated AAR to your Android module's `build.gradle.kts`:\n\n```kotlin\ndependencies {{\n    implementation(\"{}:{}-android:VERSION\")\n}}\n```",
                    config.kotlin_package(),
                    module
                ),
                format!(
                    "```kotlin\nimport {}.{}\n\n// The bundled native library is loaded via System.loadLibrary().\n```",
                    config.kotlin_package(),
                    to_pascal_case(&config.name)
                ),
                "kotlin-android",
            )
        }
        Language::Swift => (
            "Swift",
            format!(
                "Add to `Package.swift`:\n\n```swift\n.package(url: \"<repo-url>\", from: \"{}\")\n```",
                config.name
            ),
            "```swift\n// Phase 2: Swift bindings via swift-bridge. Skeleton only.\n```".to_string(),
            "swift",
        ),
        Language::Dart => (
            "Dart",
            format!(
                "Add to `pubspec.yaml`:\n\n```yaml\ndependencies:\n  {}:\n    git: <repo-url>\n```",
                config.name.replace('-', "_")
            ),
            "```dart\n// Phase 2: Dart bindings via flutter_rust_bridge. Skeleton only.\n```".to_string(),
            "dart",
        ),
        Language::Gleam => {
            let app = gleam_app_name(config);
            (
                "Gleam",
                format!("```sh\ngleam add {app}\n```"),
                format!(
                    "```gleam\nimport {app}\n\n// Call functions exported by the generated module.\n// The NIF is loaded via `@external(erlang, \"{}\", ...)`.\n```",
                    gleam_nif_module(config)
                ),
                "gleam",
            )
        }
        Language::C | Language::Jni | Language::Zig => {
            let module = config.zig_module_name();
            (
                "Zig",
                format!(
                    "Add to `build.zig.zon`:\n\n```zig\n.dependencies = .{{\n    .{module} = .{{ .url = \"<tarball-url>\" }},\n}};\n```"
                ),
                format!(
                    "```zig\nconst {module} = @import(\"{module}\");\n\n// Call generated wrapper functions; strings allocated by the FFI must\n// be released with `{module}._free_string`.\n```"
                ),
                "zig",
            )
        }
    };

    let content = format!(
        r#"# {name} - {lang_display} Bindings

{description}

## Installation

{install}

## Quick Start

{example}

## Documentation

For full documentation, see the [{name} repository]({repository}).

## License

See the [LICENSE]({repository}/blob/main/LICENSE) file in the root repository.
"#,
        name = name,
        lang_display = lang_display,
        description = description,
        install = install_instructions,
        example = example_code,
        repository = repository,
    );

    // Use the readme config output pattern if provided, otherwise default.
    // Node and Rust publish from their crate directories, not packages/ stubs.
    let path = match lang {
        Language::Ffi => PathBuf::from(format!("crates/{}-ffi/README.md", name)),
        Language::Wasm => PathBuf::from(format!("crates/{}-wasm/README.md", name)),
        Language::Node => PathBuf::from(format!("crates/{}-node/README.md", name)),
        Language::Rust => PathBuf::from(format!("crates/{}/README.md", name)),
        _ => PathBuf::from(format!("packages/{}/README.md", dir_name)),
    };

    Ok(GeneratedFile {
        path,
        content,
        generated_header: false,
    })
}

/// Convert snake_case to camelCase. Used to format function names in README examples.
pub(super) fn to_camel(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut upper_next = false;
    for (i, ch) in s.chars().enumerate() {
        if ch == '_' {
            upper_next = true;
        } else if upper_next {
            result.extend(ch.to_uppercase());
            upper_next = false;
        } else if i == 0 {
            result.extend(ch.to_lowercase());
        } else {
            result.push(ch);
        }
    }
    result
}

/// Capitalize the first character of a string.
pub(super) fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().to_string() + chars.as_str(),
    }
}
