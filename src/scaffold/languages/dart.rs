use crate::backends::dart::naming::{dart_frb_version, dart_style};
use crate::core::backend::GeneratedFile;
use crate::core::config::{DartStyle, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use crate::core::template_versions::{pub_dev, toolchain};
use crate::scaffold::scaffold_meta;
use std::path::PathBuf;

pub(crate) fn scaffold_dart(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let version = &api.version;
    let pubspec_name = config.dart_pubspec_name();
    let module_name = api.crate_name.replace('-', "_");

    let flutter_rust_bridge = dart_frb_version(config);
    let dart_sdk = toolchain::DART_SDK_CONSTRAINT;
    let test_package = pub_dev::TEST_PACKAGE;
    let lints = pub_dev::LINTS;
    let ffi_package = pub_dev::FFI_PACKAGE;
    let freezed_annotation = pub_dev::FREEZED_ANNOTATION;
    let json_annotation = pub_dev::JSON_ANNOTATION;
    let freezed = pub_dev::FREEZED;
    let build_runner = pub_dev::BUILD_RUNNER;
    let json_serializable = pub_dev::JSON_SERIALIZABLE;
    let native_assets_cli = pub_dev::NATIVE_ASSETS_CLI;
    let style = dart_style(config);

    let dependency_block = match style {
        DartStyle::Frb => format!(
            r#"  # FRB runtime is pure-Dart; works in both Flutter and server-Dart contexts.
  flutter_rust_bridge: ^{flutter_rust_bridge}
  # FRB codegen-2.x emits `@freezed` sealed classes annotated with these.
  freezed_annotation: '{freezed_annotation}'
  json_annotation: '{json_annotation}'
"#,
            flutter_rust_bridge = flutter_rust_bridge,
            freezed_annotation = freezed_annotation,
            json_annotation = json_annotation,
        ),
        DartStyle::Ffi => format!(
            r#"  # Raw dart:ffi bindings use package:ffi for native memory helpers.
  ffi: '{ffi_package}'
  # Native-assets build hook resolves the FFI shared library at consumer build time (Dart 3.0+).
  native_assets_cli: '{native_assets_cli}'
  # Product-type DTOs use @freezed annotation for code generation.
  freezed_annotation: '{freezed_annotation}'
  json_annotation: '{json_annotation}'
"#,
            ffi_package = ffi_package,
            native_assets_cli = native_assets_cli,
            freezed_annotation = freezed_annotation,
            json_annotation = json_annotation,
        ),
    };
    let dev_dependency_block = match style {
        DartStyle::Frb => format!(
            r#"  # Required by flutter_rust_bridge_codegen 2.x for sealed classes.
  freezed: '{freezed}'
  build_runner: '{build_runner}'
  json_serializable: '{json_serializable}'
"#,
            freezed = freezed,
            build_runner = build_runner,
            json_serializable = json_serializable,
        ),
        DartStyle::Ffi => format!(
            r#"  # Required for product-type DTO code generation (@freezed annotation).
  freezed: '{freezed}'
  build_runner: '{build_runner}'
  json_serializable: '{json_serializable}'
"#,
            freezed = freezed,
            build_runner = build_runner,
            json_serializable = json_serializable,
        ),
    };

    let repository_line = meta
        .configured_repository
        .as_deref()
        .map(|repository| format!("repository: {repository}\n"))
        .unwrap_or_default();
    // Only emit `homepage:` when it is non-empty to keep pubspec.yaml clean.
    let homepage_line = if meta.homepage.is_empty() {
        String::new()
    } else {
        format!("homepage: {}\n", meta.homepage)
    };

    // Host-native capsule (Language) passthrough. Dart has no idiomatic high-level
    // tree-sitter Language wrapper, so the binding returns the raw `Pointer<TSLanguage>`
    // via dart:ffi and normally needs no extra package. A capsule entry with a non-empty
    // `package` still injects a pub dependency line for consumers that wire their own wrapper.
    let capsule_dependency_lines: String = {
        let mut deps: Vec<(String, String)> = config
            .dart
            .as_ref()
            .map(|c| {
                c.capsule_types
                    .values()
                    .filter(|cap| !cap.package.is_empty())
                    .map(|cap| {
                        let ver = if cap.package_version.is_empty() {
                            "any".to_string()
                        } else {
                            cap.package_version.clone()
                        };
                        (cap.package.clone(), ver)
                    })
                    .collect()
            })
            .unwrap_or_default();
        deps.sort();
        deps.dedup();
        deps.iter().map(|(pkg, ver)| format!("  {pkg}: '{ver}'\n")).collect()
    };

    let pubspec_yaml = format!(
        r#"name: {name}
description: {description}
version: {version}
{repository_line}{homepage_line}environment:
  sdk: '{dart_sdk}'
executables:
  download_libs:
dependencies:
  http: '^1.1.0'
{capsule_dependency_lines}{dependency_block}dev_dependencies:
  test: '{test_package}'
  lints: '{lints}'
{dev_dependency_block}"#,
        name = pubspec_name,
        description = meta.description,
        version = version,
        repository_line = repository_line,
        homepage_line = homepage_line,
        capsule_dependency_lines = capsule_dependency_lines,
    );

    let generated_dir = format!("lib/src/{module_name}_bridge_generated/**");

    let analysis_options_yaml = format!(
        r#"include: package:lints/recommended.yaml

analyzer:
  exclude:
    - lib/src/frb/**
    - {generated_dir}
    - example/**
    - lib/src/traits.dart

linter:
  rules:
    - avoid_empty_else
    - avoid_print
    - avoid_relative_lib_imports
    - avoid_returning_this
    - avoid_slow_async_io
    - cancel_subscriptions
    - close_sinks
    - comment_references
    - control_flow_in_finally
    - empty_statements
    - hash_and_equals
    - literal_only_boolean_expressions
    - no_adjacent_strings_in_list
    - no_duplicate_case_values
    - prefer_void_to_null
    - throw_in_finally
    - unnecessary_statements
    - unrelated_type_equality_checks
"#
    );

    let gitignore = ".dart_tool/\nbuild/\npubspec.lock\n";

    let pubignore = "android/\nios/\nblobs/\nlib/src/native/\nrust/\nexample/\ntest/\n";

    let test_dart = r#"import 'package:test/test.dart';

void main() {
  test('placeholder', () {
    expect(1 + 1, equals(2));
  });
}
"#
    .to_string();

    let crate_name = &api.crate_name;
    let build_commands = match style {
        DartStyle::Frb => format!(
            r#"cargo build -p {crate_name}-dart
flutter_rust_bridge_codegen generate
dart pub get
dart analyze
dart test"#
        ),
        DartStyle::Ffi => r#"cargo build --release -p {crate_name}-ffi
dart pub get
dart analyze
dart test"#
            .replace("{crate_name}", crate_name),
    };
    let license_section = meta
        .license
        .as_deref()
        .map(|license| format!("\n## License\n\n{license}\n"))
        .unwrap_or_default();

    let readme = format!(
        r#"# {pubspec_name}

{description}

## Installation

Add to your `pubspec.yaml`:

```yaml
dependencies:
  {pubspec_name}: ^{version}
```

Then run:

```sh
dart pub get
```

## Building

From the repository root:

```sh
{build_commands}
```
"#,
        pubspec_name = pubspec_name,
        description = meta.description,
        version = version,
    ) + &license_section;

    let editorconfig = "[*]\ncharset = utf-8\nend_of_line = lf\ninsert_final_newline = true\n\n[*.dart]\nindent_style = space\nindent_size = 2\n";

    // pub.dev requires a CHANGELOG.md in the package root. Emit a minimal seed
    // entry keyed to the current version. This file has generated_header: false
    // so it is a create-once seed — users update it before publishing.
    let changelog = format!(
        "# Changelog\n\nAll notable changes to this package will be documented in this file.\n\n## {version}\n\n- Initial release.\n",
        version = version,
    );

    let example_dart = format!(
        r#"import 'package:{pubspec_name}' as {module_name};

void main() {{
  print('Example: {pubspec_name} loaded successfully');
  // Add your API calls here after code generation
}}
"#,
        pubspec_name = pubspec_name,
        module_name = module_name,
    );

    let mut files = vec![
        GeneratedFile {
            path: PathBuf::from("packages/dart/pubspec.yaml"),
            content: pubspec_yaml,
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/dart/analysis_options.yaml"),
            content: analysis_options_yaml,
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/dart/.gitignore"),
            content: gitignore.to_string(),
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/dart/.pubignore"),
            content: pubignore.to_string(),
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from(format!("packages/dart/test/{module_name}_test.dart")),
            content: test_dart,
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/dart/.editorconfig"),
            content: editorconfig.to_string(),
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/dart/README.md"),
            content: readme,
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from(format!("packages/dart/example/{module_name}_example.dart")),
            content: example_dart,
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/dart/CHANGELOG.md"),
            content: changelog,
            generated_header: false,
        },
    ];

    if matches!(style, DartStyle::Ffi) {
        let build_dart = format!(
            r#"// Dart 3.0+ native-assets build hook.
// Resolves the FFI shared library produced by `cargo build --release -p {crate_name}-ffi`
// and bundles it into the consumer's Dart application at build time.
// See: https://dart.dev/interop/c-interop#native-assets

import 'dart:io' as io;
import 'package:native_assets_cli/native_assets_cli.dart';

const _crateName = '{crate_name}';
const _packageName = '{pubspec_name}';

Future<void> main(List<String> args) async {{
  await build(args, (input, output) async {{
    final libFile = input.config.targetOS.dylibFileName(_crateName);
    final repoRoot = _findRepoRoot(io.Directory.current);
    final candidates = <io.File>[
      io.File('${{repoRoot.path}}/target/release/$libFile'),
      io.File('${{repoRoot.path}}/crates/${{_crateName}}-ffi/target/release/$libFile'),
      io.File('${{repoRoot.path}}/packages/dart/rust/target/release/$libFile'),
    ];
    for (final candidate in candidates) {{
      if (candidate.existsSync()) {{
        output.addAsset(NativeCodeAsset(
          package: _packageName,
          name: '${{_packageName}}.dart',
          file: candidate.uri,
          linkMode: DynamicLoadingBundled(),
          os: input.config.targetOS,
          architecture: input.config.targetArchitecture,
        ));
        return;
      }}
    }}
    throw StateError(
      'Native library $libFile not found. '
      'Build it with: cargo build --release -p ${{_crateName}}-ffi',
    );
  }});
}}

io.Directory _findRepoRoot(io.Directory start) {{
  io.Directory current = start;
  while (current.path != current.parent.path) {{
    if (io.File('${{current.path}}/Cargo.toml').existsSync() &&
        io.Directory('${{current.path}}/.git').existsSync()) {{
      return current;
    }}
    current = current.parent;
  }}
  return start;
}}
"#,
            crate_name = crate_name,
            pubspec_name = pubspec_name,
        );
        files.push(GeneratedFile {
            path: PathBuf::from("packages/dart/hook/build.dart"),
            content: build_dart,
            generated_header: false,
        });
    }

    Ok(files)
}
