use crate::scaffold_meta;
use alef_backend_dart::naming::dart_frb_version;
use alef_core::backend::GeneratedFile;
use alef_core::config::ResolvedCrateConfig;
use alef_core::ir::ApiSurface;
use alef_core::template_versions::{pub_dev, toolchain};
use std::path::PathBuf;

pub(crate) fn scaffold_dart(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let version = &api.version;
    let pubspec_name = config.dart_pubspec_name();

    let flutter_rust_bridge = dart_frb_version(config);
    let dart_sdk = toolchain::DART_SDK_CONSTRAINT;
    let test_package = pub_dev::TEST_PACKAGE;
    let lints = pub_dev::LINTS;

    // flutter_rust_bridge is listed under `dependencies:` because the generated
    // Dart wrapper imports its runtime types. For pure-Dart (non-Flutter)
    // consumers the FRB pub package is plain Dart and pulls no Flutter SDK; for
    // Flutter consumers the same dep resolves to the Flutter-augmented variant.
    // No conditional dep block is needed — the package author can override
    // by setting `[dart] frb_version` to a `git:` reference if a forked variant
    // is required.
    let pubspec_yaml = format!(
        r#"name: {name}
description: {description}
version: {version}
environment:
  sdk: '{dart_sdk}'
dependencies:
  # FRB runtime is pure-Dart; works in both Flutter and server-Dart contexts.
  flutter_rust_bridge: '{flutter_rust_bridge}'
  # FRB codegen-2.x emits `@freezed` sealed classes annotated with these.
  freezed_annotation: '^2.4.0'
  json_annotation: '^4.9.0'
dev_dependencies:
  test: '{test_package}'
  lints: '{lints}'
  # Required by flutter_rust_bridge_codegen 2.x for sealed classes.
  freezed: '^2.5.0'
  build_runner: '^2.4.0'
  json_serializable: '^6.8.0'
"#,
        name = pubspec_name,
        description = meta.description,
        version = version,
    );

    // Linter rule list intentionally excludes lints removed in Dart 3.x:
    //   - avoid_returning_null, avoid_returning_null_for_future (removed in 3.3)
    //   - invariant_booleans (removed in 3.0)
    //   - iterable_contains_unrelated_type, list_remove_unrelated_type (removed in 3.0)
    //
    // The `analyzer.exclude` block silences analysis on flutter_rust_bridge-generated
    // files: those use inline-class extension types and reference unignored frb
    // types that the analyzer can't resolve until codegen has run.
    let analysis_options_yaml = "include: package:lints/recommended.yaml\n\nanalyzer:\n  exclude:\n    - lib/src/frb/**\n    - lib/src/kreuzberg_bridge_generated/**\n    - example/**\n    - lib/src/traits.dart\n\nlinter:\n  rules:\n    - avoid_empty_else\n    - avoid_print\n    - avoid_relative_lib_imports\n    - avoid_returning_this\n    - avoid_slow_async_io\n    - cancel_subscriptions\n    - close_sinks\n    - comment_references\n    - control_flow_in_finally\n    - empty_statements\n    - hash_and_equals\n    - literal_only_boolean_expressions\n    - no_adjacent_strings_in_list\n    - no_duplicate_case_values\n    - prefer_void_to_null\n    - throw_in_finally\n    - unnecessary_statements\n    - unrelated_type_equality_checks\n";

    let gitignore = ".dart_tool/\nbuild/\npubspec.lock\n";

    let module_name = api.crate_name.replace('-', "_");

    let test_dart = r#"import 'package:test/test.dart';

void main() {
  test('placeholder', () {
    expect(1 + 1, equals(2));
  });
}
"#
    .to_string();

    let crate_name = &api.crate_name;
    let building_md = format!(
        r#"# Building {crate_name} Dart bindings

## Prerequisites

Install the flutter_rust_bridge codegen tool (one-time setup):

```sh
cargo install flutter_rust_bridge_codegen
```

## Build steps

1. Build the Rust binding crate:

   ```sh
   cargo build -p {crate_name}-dart
   ```

2. Run the FRB codegen to generate Dart bridge files:

   ```sh
   flutter_rust_bridge_codegen generate
   ```

   Alternatively, use alef which runs this step automatically via the configured
   post-build hook:

   ```sh
   alef build --lang=dart
   ```

3. Fetch Dart dependencies and run the test suite:

   ```sh
   dart pub get
   dart test
   ```
"#
    );

    let editorconfig = "[*]\ncharset = utf-8\nend_of_line = lf\ninsert_final_newline = true\n\n[*.dart]\nindent_style = space\nindent_size = 2\n";

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
cargo build -p {crate_name}-dart
flutter_rust_bridge_codegen generate
dart pub get
dart analyze
dart test
```

For detailed build instructions, see [BUILDING.md](BUILDING.md).

## License

{license}
"#,
        pubspec_name = pubspec_name,
        description = meta.description,
        crate_name = crate_name,
        version = version,
        license = meta.license,
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
            content: analysis_options_yaml.to_string(),
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
            path: PathBuf::from("packages/dart/BUILDING.md"),
            content: building_md,
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
