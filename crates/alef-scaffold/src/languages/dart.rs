use crate::scaffold_meta;
use alef_backend_dart::naming::{dart_frb_version, dart_style};
use alef_core::backend::GeneratedFile;
use alef_core::config::{DartStyle, ResolvedCrateConfig};
use alef_core::ir::ApiSurface;
use alef_core::template_versions::{pub_dev, toolchain};
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
    let style = dart_style(config);

    let dependency_block = match style {
        DartStyle::Frb => format!(
            r#"  # FRB runtime is pure-Dart; works in both Flutter and server-Dart contexts.
  flutter_rust_bridge: '{flutter_rust_bridge}'
  # FRB codegen-2.x emits `@freezed` sealed classes annotated with these.
  freezed_annotation: '{freezed_annotation}'
  json_annotation: '{json_annotation}'
"#
        ),
        DartStyle::Ffi => format!(
            r#"  # Raw dart:ffi bindings use package:ffi for native memory helpers.
  ffi: '{ffi_package}'
"#
        ),
    };
    let dev_dependency_block = match style {
        DartStyle::Frb => format!(
            r#"  # Required by flutter_rust_bridge_codegen 2.x for sealed classes.
  freezed: '{freezed}'
  build_runner: '{build_runner}'
  json_serializable: '{json_serializable}'
"#
        ),
        DartStyle::Ffi => String::new(),
    };

    let pubspec_yaml = format!(
        r#"name: {name}
description: {description}
version: {version}
environment:
  sdk: '{dart_sdk}'
dependencies:
{dependency_block}dev_dependencies:
  test: '{test_package}'
  lints: '{lints}'
{dev_dependency_block}"#,
        name = pubspec_name,
        description = meta.description,
        version = version,
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

## License

{license}
"#,
        pubspec_name = pubspec_name,
        description = meta.description,
        version = version,
        license = meta.license,
    );

    let editorconfig = "[*]\ncharset = utf-8\nend_of_line = lf\ninsert_final_newline = true\n\n[*.dart]\nindent_style = space\nindent_size = 2\n";

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

    let github_workflow = r#"name: Dart

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

jobs:
  test:
    runs-on: ubuntu-latest
    defaults:
      run:
        working-directory: packages/dart
    steps:
      - uses: actions/checkout@v4
      - name: Set up Dart
        uses: dart-lang/setup-dart@v1
        with:
          sdk: stable
      - name: Get Dart dependencies
        run: dart pub get
      - name: Analyze Dart code
        run: dart analyze
      - name: Run tests
        run: dart test
"#
    .to_string();

    Ok(vec![
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
            path: PathBuf::from(".github/workflows/dart.yml"),
            content: github_workflow,
            generated_header: false,
        },
    ])
}
