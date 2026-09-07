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
        Language::Python => {
            let module = config.python_module_name().trim_start_matches('_').to_string();
            let example_body = api
                .functions
                .iter()
                .find(|function| !function.binding_excluded)
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
                .iter()
                .find(|function| !function.binding_excluded)
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
            let wrapper_class = crate::codegen::naming::csharp_wrapper_class_name(&api.crate_name, &ns);
            let example_body = api
                .functions
                .iter()
                .find(|function| !function.binding_excluded)
                .map(|f| {
                    format!(
                        "// var result = {wrapper_class}.{method}(...);\n// See the main repository's docs for full usage.",
                        method = crate::codegen::naming::to_csharp_name(&f.name),
                    )
                })
                .unwrap_or_else(|| format!("// {example_pointer}"));
            (
                "C#",
                format!("```bash\ndotnet add package {ns}\n```"),
                format!("```csharp\nusing {ns};\n\n{example_body}\n```"),
                "csharp",
            )
        }
        Language::Ffi => {
            let header = config.ffi_header_name();
            // A cargo `staticlib`/`cdylib` is named `lib<lib name>.{a,dylib,so}`, where the lib
            // name defaults to the FFI crate's OWN package name with `-` mapped to `_` — never
            // the alef crate name, and never a hyphenated string. `ffi_lib_name()` is that value
            // (`[crates.ffi] lib_name`, else the configured `[crates.output] ffi` directory), and
            // is already what the Go/Java/Kotlin/Dart backends link against. ~keep
            let lib_name = config.ffi_lib_name();
            let example_body = format!("    // {example_pointer}");
            (
                "FFI (C/C++)",
                format!(
                    "Link against `lib{lib_name}` and include `{header}`.\n\nSee the build instructions in the main repository.",
                ),
                format!("```c\n#include \"{header}\"\n\nint main(void) {{\n{example_body}\n    return 0;\n}}\n```"),
                "ffi",
            )
        }
        Language::Wasm => {
            // The published npm name, not `{crate name}-wasm`. The npm scope is not derivable
            // from the crate name — `wasm_package_name()` defaults from `node_package_name()`,
            // so a consumer whose node package is scoped resolves to `@xberg-io/<crate>-wasm`
            // where the crate-name template gives the unscoped `<crate>-wasm`, which installs
            // nothing. This is the Wasm analogue of the `lib{name}_ffi` defect in the FFI arm
            // above: wrong even where the README path is right by coincidence. ~keep
            let pkg = config.wasm_package_name();
            let example_body = format!("// {example_pointer}");
            (
                "WebAssembly",
                format!("```bash\nnpm install {pkg}\n```"),
                format!("```javascript\nimport init from '{pkg}';\n\nawait init();\n{example_body}\n```"),
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

    // `dir_name` is deliberately not `paths::lang_dir_name(lang)`: this generator files C and JNI
    // READMEs under `packages/zig/`, which that function maps to `packages/c/`. Only the
    // crate-hosted languages share their path rule with `paths.rs`. ~keep
    let derived_path = super::paths::crate_readme_path(config, lang)
        .unwrap_or_else(|| PathBuf::from(format!("packages/{dir_name}/README.md")));
    let path = configured_output_path(config, lang)?.unwrap_or(derived_path);

    // See the matching `~keep` note in `template.rs` for why README output
    // gets the same self-embedded HTML-comment marker as docs pages: `.md`
    // cannot carry alef's usual comment-based header, so this is the only way
    // the write-time ownership guard can prove ownership from content alone.
    let content = crate::docs::with_html_header(content, "alef readme");

    Ok(GeneratedFile {
        path,
        content,
        generated_header: true,
    })
}

/// The output path the config asks this README to take, if it asks for one.
///
/// A language entry that carries only `output_path` — no `template` — never reaches the
/// templated route, because `try_render_configured_readme` returns `None` for the whole crate
/// when `crates.readme.template_dir` is unset (and for a single language when its entry or
/// template file is missing), which routes the language here. That makes this generator the
/// only place left that can honour the configured path, and it used to derive one instead:
/// `crates/{stem}` or `packages/{lang}`, written wherever the derivation happened to land. The
/// derivation usually agrees with the configured value, which is why the defect survived —
/// agreement is coincidence, not design, exactly as it was for the FFI directory rule. The
/// precedence itself stays in `paths.rs` so the templated and fallback routes cannot drift. ~keep
fn configured_output_path(config: &ResolvedCrateConfig, lang: Language) -> anyhow::Result<Option<PathBuf>> {
    let Some(readme_cfg) = &config.readme else {
        return Ok(None);
    };
    let workspace_root = config.workspace_root.clone().unwrap_or_else(|| PathBuf::from("."));
    let entry = super::template::language_entry(readme_cfg, &workspace_root, super::paths::lang_code(lang))?
        .unwrap_or(serde_json::Value::Null);
    Ok(super::paths::configured_output_path(readme_cfg, lang, &entry))
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
