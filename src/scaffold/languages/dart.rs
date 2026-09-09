use crate::backends::dart::ident::dart_safe_ident;
use crate::backends::dart::naming::{dart_frb_version, dart_style};
use crate::codegen::shared::binding_fields;
use crate::core::backend::GeneratedFile;
use crate::core::config::{DartStyle, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, PrimitiveType, TypeDef, TypeRef};
use crate::core::template_versions::{pub_dev, toolchain};
use crate::scaffold::{readme_language_configured, scaffold_meta};
use anyhow::Context as _;
use heck::ToLowerCamelCase;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

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
    let http_package = pub_dev::HTTP_PACKAGE;
    let crypto_package = pub_dev::CRYPTO;
    let style = dart_style(config);

    let dependency_block = match style {
        DartStyle::Frb => format!(
            r#"  # FRB runtime is pure-Dart; works in both Flutter and server-Dart contexts.
  # Pinned exactly, not with a caret: flutter_rust_bridge asserts at RustLib.init() that the
  # Dart runtime version equals the version its codegen ran as, and the sibling Rust crate
  # carries an `=` pin on the same constant. A caret lets `dart pub get` satisfy a stale
  # pubspec.lock with an older minor and hand the app a "codegen version (X) should be the same
  # as runtime version (Y)" Bad state at first call -- a range on one side of an equality
  # requirement is drift waiting to happen. ~keep
  flutter_rust_bridge: {flutter_rust_bridge}
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
    let homepage_line = if meta.homepage.is_empty() {
        String::new()
    } else {
        format!("homepage: {}\n", meta.homepage)
    };

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
  http: '{http_package}'
  # SHA-256 verification of downloaded native-library release assets.
  crypto: '{crypto_package}'
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
        http_package = http_package,
        crypto_package = crypto_package,
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

    // NOTE: do NOT exclude lib/src/native/ or *.so/*.dylib/*.dll here. Native FFI
    // libraries are staged into lib/src/native/<rid>/ at publish time and MUST ship in
    // the pub.dev tarball; .pubignore fully replaces git-based file listing, so excluding
    // them silently strips the natives and consumers cannot load the FFI library.
    let pubignore = "android/\nios/\nblobs/\nrust/\nexample/\ntest/\n";

    // `package:` URIs require at least one path segment after the package name.
    // FRB style exports the public API through the default barrel file at
    // `lib/{module_name}.dart` (see `barrel_name` in gen_bindings::generate_bindings,
    // which defaults to the same crate-derived module name used here). FFI style has
    // no barrel — the re-export wrapper lives at `lib/src/{module_name}.dart`
    // (see gen_ffi::emit). ~keep
    let package_import_path = match style {
        DartStyle::Frb => format!("{pubspec_name}/{module_name}.dart"),
        DartStyle::Ffi => format!("{pubspec_name}/src/{module_name}.dart"),
    };

    let test_dart = scaffold_dart_test(api, config, &module_name, &package_import_path);

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

    let changelog = format!(
        "# Changelog\n\nAll notable changes to this package will be documented in this file.\n\n## {version}\n\n- Initial release.\n",
        version = version,
    );

    let example_dart = format!(
        r#"import 'package:{package_import_path}' as {module_name};

void main() {{
  print('Example: {pubspec_name} loaded successfully');
  // Add your API calls here after code generation
}}
"#,
        package_import_path = package_import_path,
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
    // See the matching comment in `scaffold_swift`: once `[crates.readme.languages.dart]`
    // is configured, the README module owns this path end-to-end, and scaffold must not
    // compete with it as a second writer (#555). Inserted at its original position
    // (after `.editorconfig`, before the example) rather than appended, so file order
    // is unchanged for languages that still rely on this placeholder. ~keep
    if !readme_language_configured(config, "dart") {
        files.insert(
            6,
            GeneratedFile {
                path: PathBuf::from("packages/dart/README.md"),
                content: readme,
                generated_header: false,
            },
        );
    }

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

/// Build the seed content for `test/{module_name}_test.dart`.
///
/// `write_scaffold_files_report` treats `generated_header: false` as create-only, so once
/// a real suite exists at this path alef never overwrites it; this only ever seeds a fresh
/// project. The seed must not be vacuous — `expect(1 + 1, equals(2))` compiles and passes
/// no matter what the generated API looks like, exactly the "0 assertions, silently green"
/// defect the zig test-module fix (`scaffold_zig_test`) and the Swift seed
/// (`scaffold_swift_test`) already closed one layer down. So, mirroring `scaffold_swift_test`'s
/// approach, this asserts against the *real*, currently-generated API surface (`api`), in
/// order of how strong a check is safely synthesizable without duplicating the Dart binding
/// emitter's full type-mapping surface:
///
/// 1. A visible, non-opaque DTO (`has_serde`, struct, all fields plain primitives/`String`,
///    no optional/cfg-gated fields) is literal-constructed twice with identical field values
///    and compared for equality — it fails to compile if the generated constructor drops or
///    renames a field, and fails at runtime if the generated (freezed) value-equality stops
///    being field-based, not just on a missing symbol.
/// 2. Otherwise, any other visible type or enum is referenced bare (`{module}.Name`), forcing
///    the analyzer to resolve it — weaker than the round trip (construction/field shape isn't
///    checked) but still a real, falsifiable fact about the generated output.
/// 3. Only when the API surface is genuinely empty (e.g. scaffolding before any Rust code
///    exists) does this fall back to the harmless `1 + 1 == 2` placeholder that merely forces
///    the package import to resolve — there is nothing else to assert against yet, and once
///    real items exist this file is never regenerated over. ~keep
fn scaffold_dart_test(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    module_name: &str,
    package_import_path: &str,
) -> String {
    let exclude_types = dart_binding_exclusions(config);

    let round_trip_candidate = api
        .types
        .iter()
        .filter(|t| dart_type_is_visible(t, &exclude_types))
        .filter(|t| !t.is_opaque && t.has_serde && !t.has_stripped_cfg_fields)
        .find_map(|t| simple_dart_fields(t).map(|fields| (t, fields)));
    if let Some((ty, fields)) = round_trip_candidate {
        return dart_equality_round_trip_test(module_name, package_import_path, &ty.name, &fields);
    }

    let visible_type_name = api
        .types
        .iter()
        .filter(|t| dart_type_is_visible(t, &exclude_types))
        .map(|t| t.name.clone())
        .next();
    let visible_enum_name = || {
        api.enums
            .iter()
            .filter(|e| !e.binding_excluded && !exclude_types.contains(&e.name))
            .map(|e| e.name.clone())
            .next()
    };
    if let Some(name) = visible_type_name.or_else(visible_enum_name) {
        return dart_type_reference_test(module_name, package_import_path, &name);
    }

    dart_placeholder_test(module_name, package_import_path)
}

/// Repair a pre-existing `test/{module}_test.dart` that is still the vacuous
/// `expect(1 + 1, equals(2))` placeholder this scaffold used to emit, now that
/// [`scaffold_dart_test`] can generate a real assertion against the actual API surface.
///
/// Mirrors `migrate_build_zig_test_target`'s strategy (`src/scaffold/languages/zig.rs`) in
/// spirit and for the same reason: `test/*_test.dart` is `generated_header: false`
/// (create-only) on a markable `.dart` extension, so a fresh clone can never have this
/// content fix reach it through the normal write path at all, no matter what flag is
/// passed.
///
/// Detection is a *vacuity signature*, not a byte-for-byte template match, because the
/// placeholder's exact bytes have already drifted across the repos this exists to fix: the
/// current generator (`dart_placeholder_test`) wraps the assertion in a three-line rationale
/// comment, while the oldest shape actually found on disk (consumer A's
/// `packages/dart/test/{module}_test.dart`) has neither that comment nor a
/// `package:` import at all — just `import 'package:test/test.dart';` and the bare
/// `test('placeholder', ...) { expect(1 + 1, equals(2)); }`. A byte-match constant validated
/// only against the current generator's own output, as an earlier revision of this function
/// did, matches none of the historical shapes it exists to repair. So instead this fires
/// only when the file's *sole* assertion is the tautology: it contains
/// `expect(1 + 1, equals(2))`, contains `test('placeholder'`, and contains **exactly one**
/// `expect(` and **exactly one** `test(` in the whole file — i.e. there is nothing else in
/// it to lose. Any hand-written suite (more assertions, more tests, a renamed test) fails
/// that count and is left completely untouched, verified against all three real consumer
/// trees (consumer A: 1/1, fires; consumer B: 3 `expect(`/1 `test(`, hand-written,
/// does not fire; consumer C: 17 `expect(`/18 `test(`, hand-written, does not
/// fire). Idempotent: the freshly generated replacement itself never matches this signature
/// once real API surface exists, and even when the surface is still empty (replacement is
/// itself the placeholder), the byte-equality check below makes the second pass a no-op. ~keep
pub(crate) fn migrate_dart_placeholder_test(
    base_dir: &Path,
    relative_path: &Path,
    replacement: &str,
) -> anyhow::Result<bool> {
    let path = crate::cli::pipeline::generate::write::contained_output_path(base_dir, relative_path)?;
    let Ok(existing) = std::fs::read_to_string(&path) else {
        return Ok(false);
    };
    if !is_vacuous_dart_placeholder(&existing) {
        return Ok(false);
    }
    if existing == replacement {
        return Ok(false);
    }

    let parent = path.parent().context("dart test path has no parent directory")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("failed to create temporary file in {}", parent.display()))?;
    std::io::Write::write_all(&mut temporary, replacement.as_bytes())
        .with_context(|| format!("failed to write temporary file for {}", path.display()))?;
    temporary
        .persist(&path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to replace {}", path.display()))?;
    // Fires only after the replace above already succeeded: a completed self-heal, not an
    // outstanding problem. ~keep
    tracing::info!(
        path = %path.display(),
        "repaired pre-existing test/*_test.dart: replaced the vacuous 1 + 1 == 2 placeholder \
         with a real assertion against the generated API"
    );
    Ok(true)
}

/// The vacuity signature behind [`migrate_dart_placeholder_test`]: true when `content`'s
/// only assertion is the `1 + 1 == 2` tautology, regardless of which placeholder-template
/// revision emitted it (see that function's doc for why a byte-match template comparison
/// does not survive template drift across the real repos this targets).
fn is_vacuous_dart_placeholder(content: &str) -> bool {
    content.contains("expect(1 + 1, equals(2))")
        && content.contains("test('placeholder'")
        && content.matches("expect(").count() == 1
        && content.matches("test(").count() == 1
}

/// The exact `.pubignore` body [`scaffold_dart`] emitted before the fix that stopped excluding
/// native FFI libraries from the published pub.dev tarball.
const STALE_PUBIGNORE: &str = "android/\nios/\nblobs/\nlib/src/native/\nrust/\nexample/\ntest/\n*.so\n*.dylib\n*.dll\n";

/// Repair a pre-existing `packages/dart/.pubignore` that still excludes `lib/src/native/` and
/// `*.so`/`*.dylib`/`*.dll` — the exact defect fixed in [`scaffold_dart`]'s `pubignore` literal.
///
/// `.pubignore` is `generated_header: false` (create-only), so a repo scaffolded before that fix
/// keeps excluding its own native FFI libraries from the published pub.dev tarball forever:
/// `.pubignore` fully replaces git-based file listing, so those entries silently strip the
/// CI-staged natives from every release, leaving consumers unable to load the FFI library at
/// all — not a degraded build, a completely broken one. Unlike the placeholder-test migrators,
/// `.pubignore` carries no per-project variables at all (no crate name, no module name — every
/// scaffolded Dart package gets the identical literal), so an exact byte match against the one
/// known-bad constant is both sufficient and maximally conservative: any consumer edit at all —
/// adding an entry, reordering lines, even trailing-whitespace drift — fails the match and is
/// left completely untouched, exactly like `migrate_swift_placeholder_test`'s "hand-written
/// suite must survive byte for byte" guarantee. ~keep
pub(crate) fn migrate_dart_pubignore(base_dir: &Path, relative_path: &Path, replacement: &str) -> anyhow::Result<bool> {
    let path = crate::cli::pipeline::generate::write::contained_output_path(base_dir, relative_path)?;
    let Ok(existing) = std::fs::read_to_string(&path) else {
        return Ok(false);
    };
    if existing != STALE_PUBIGNORE {
        return Ok(false);
    }
    if existing == replacement {
        return Ok(false);
    }

    let parent = path.parent().context(".pubignore path has no parent directory")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("failed to create temporary file in {}", parent.display()))?;
    std::io::Write::write_all(&mut temporary, replacement.as_bytes())
        .with_context(|| format!("failed to write temporary file for {}", path.display()))?;
    temporary
        .persist(&path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to replace {}", path.display()))?;
    // Fires only after the replace above already succeeded: a completed self-heal, not an
    // outstanding problem. ~keep
    tracing::info!(
        path = %path.display(),
        "repaired pre-existing packages/dart/.pubignore: stopped excluding native FFI \
         libraries (lib/src/native/, *.so/*.dylib/*.dll) from the published package"
    );
    Ok(true)
}

/// Names excluded from Dart binding generation, mirroring `[crates.dart] exclude_types` plus
/// `[crates.ffi] exclude_types` that the real Dart binding emitter honors. Kept in sync
/// deliberately rather than shared, since this seed-picker only needs *a* safe, visible name,
/// not the emitter's exhaustive filtered set. Unlike the Swift seed's equivalent, there is no
/// `[crates.dart] exclude_fields` knob to fold in — Dart has no per-field exclusion config.
fn dart_binding_exclusions(config: &ResolvedCrateConfig) -> HashSet<String> {
    let mut exclude_types: HashSet<String> = config
        .dart
        .as_ref()
        .map(|c| c.exclude_types.iter().cloned().collect())
        .unwrap_or_default();
    if let Some(ffi) = &config.ffi {
        exclude_types.extend(ffi.exclude_types.iter().cloned());
    }
    exclude_types
}

/// A visible (non-trait, non-`binding_excluded`, not config-excluded) candidate type for the
/// scaffold seed to reference.
fn dart_type_is_visible(ty: &TypeDef, exclude_types: &HashSet<String>) -> bool {
    !ty.is_trait && !ty.binding_excluded && !exclude_types.contains(&ty.name)
}

/// A field simple enough to synthesize a literal Dart value for: a primitive or `String`,
/// never optional, never `#[cfg(...)]`-gated (whether it exists depends on active features,
/// which this scaffold-time seed cannot know).
struct SimpleDartField {
    label: String,
    literal: String,
}

/// Compute a literal-constructible field list for `ty`, or `None` when any visible field
/// falls outside the safely synthesizable subset (optional, cfg-gated, or a type other than
/// a primitive/`String` — `Named`/`Vec`/`Map`/etc. would need recursive construction this
/// seed does not attempt). Bails on the *whole type* rather than partially constructing it,
/// since the real generated constructor requires every non-optional named parameter.
fn simple_dart_fields(ty: &TypeDef) -> Option<Vec<SimpleDartField>> {
    let mut fields = Vec::new();
    for field in binding_fields(&ty.fields) {
        if field.optional || field.cfg.is_some() {
            return None;
        }
        let literal = match &field.ty {
            TypeRef::Primitive(primitive) => dart_primitive_literal(primitive),
            TypeRef::String => "'alef-scaffold'".to_string(),
            _ => return None,
        };
        fields.push(SimpleDartField {
            label: dart_safe_ident(&field.name.to_lower_camel_case()),
            literal,
        });
    }
    if fields.is_empty() { None } else { Some(fields) }
}

/// A literal Dart value for a primitive type. `bool` gets a non-default `true` and floats a
/// non-integral `1.5` so a constructor that silently drops or zeroes a field is still caught
/// by the equality check.
fn dart_primitive_literal(primitive: &PrimitiveType) -> String {
    match primitive {
        PrimitiveType::Bool => "true".to_string(),
        PrimitiveType::F32 | PrimitiveType::F64 => "1.5".to_string(),
        _ => "1".to_string(),
    }
}

/// The strongest safe check: literal-construct a visible DTO twice with identical field
/// values and assert the two instances are equal, so a generated constructor that drops or
/// renames a field, or a value-equality (`==`) implementation that stops being field-based,
/// fails `dart test` immediately instead of shipping green with a suite that asserts nothing.
fn dart_equality_round_trip_test(
    module_name: &str,
    package_import_path: &str,
    type_name: &str,
    fields: &[SimpleDartField],
) -> String {
    let init_args = fields
        .iter()
        .map(|f| format!("{}: {}", f.label, f.literal))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        r#"import 'package:test/test.dart';
import 'package:{package_import_path}' as {module_name};

void main() {{
  test('{type_name} equality holds for identical field values', () {{
    // Literal-constructs the generated `{type_name}` DTO twice with identical field
    // values and compares them for equality, so a constructor that drops/renames a
    // field, or generated equality that stops being field-based, fails `dart test`
    // immediately instead of shipping green with a suite that asserts nothing about
    // the generated API. Create-only scaffold seed. ~keep
    final a = {module_name}.{type_name}({init_args});
    final b = {module_name}.{type_name}({init_args});
    expect(a, equals(b));
  }});
}}
"#,
        package_import_path = package_import_path,
        module_name = module_name,
        type_name = type_name,
        init_args = init_args,
    )
}

/// `name` isn't a literal-constructible DTO this seed can safely construct generically, so
/// this checks the generated type or enum exists and is referenceable at compile time
/// instead.
fn dart_type_reference_test(module_name: &str, package_import_path: &str, name: &str) -> String {
    format!(
        r#"import 'package:test/test.dart';
import 'package:{package_import_path}' as {module_name};

void main() {{
  test('{module_name} exposes `{name}`', () {{
    // `{name}` isn't a literal-constructible DTO this seed can safely construct
    // generically, so this checks the generated type exists and is referenceable at
    // compile time instead. Create-only scaffold seed. ~keep
    expect({module_name}.{name}, isNotNull);
  }});
}}
"#,
        package_import_path = package_import_path,
        module_name = module_name,
        name = name,
    )
}

/// No generated API surface exists yet for this crate, so there is nothing to assert against
/// beyond the package import resolving. Once real types exist, alef never regenerates over
/// this file — it is a create-only scaffold seed.
fn dart_placeholder_test(module_name: &str, package_import_path: &str) -> String {
    format!(
        r#"import 'package:test/test.dart';
// ignore: unused_import
import 'package:{package_import_path}' as {module_name};

void main() {{
  test('placeholder', () {{
    // No generated API surface exists yet for this crate, so there is nothing to assert
    // against beyond the module resolving. Once real types exist, alef never regenerates
    // over this file -- it is a create-only scaffold seed. ~keep
    expect(1 + 1, equals(2));
  }});
}}
"#,
        package_import_path = package_import_path,
        module_name = module_name,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::NewAlefConfig;
    use crate::core::ir::{EnumDef, FieldDef};

    fn resolve_config(toml_text: &str) -> ResolvedCrateConfig {
        let cfg: NewAlefConfig = toml::from_str(toml_text).expect("valid config");
        cfg.resolve().expect("resolve").remove(0)
    }

    fn minimal_config() -> ResolvedCrateConfig {
        resolve_config(
            r#"
[workspace]
languages = ["dart"]
[[crates]]
name = "my-lib"
sources = []
"#,
        )
    }

    fn find_file<'a>(files: &'a [GeneratedFile], path: &str) -> &'a GeneratedFile {
        files
            .iter()
            .find(|f| f.path == std::path::Path::new(path))
            .unwrap_or_else(|| panic!("missing scaffolded file: {path}"))
    }

    fn simple_type(name: &str) -> TypeDef {
        TypeDef {
            name: name.to_string(),
            has_serde: true,
            fields: vec![
                FieldDef {
                    name: "count".to_string(),
                    ty: TypeRef::Primitive(PrimitiveType::U32),
                    ..Default::default()
                },
                FieldDef {
                    name: "label".to_string(),
                    ty: TypeRef::String,
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }

    /// A visible DTO whose fields are all plain primitives/`String` is literal-constructed
    /// twice and compared for equality — the strongest safe check, since it fails to
    /// compile on a dropped/renamed field and fails at runtime on broken equality, not
    /// just on a missing symbol.
    #[test]
    fn scaffold_test_round_trips_a_simple_dto() {
        let api = ApiSurface {
            types: vec![simple_type("Widget")],
            ..Default::default()
        };
        let out = scaffold_dart_test(&api, &minimal_config(), "my_lib", "my_lib/my_lib.dart");

        assert!(
            out.contains("import 'package:my_lib/my_lib.dart' as my_lib;"),
            "got:\n{out}"
        );
        assert!(
            out.contains("test('Widget equality holds for identical field values'"),
            "got:\n{out}"
        );
        assert!(
            out.contains("final a = my_lib.Widget(count: 1, label: 'alef-scaffold');"),
            "got:\n{out}"
        );
        assert!(
            out.contains("final b = my_lib.Widget(count: 1, label: 'alef-scaffold');"),
            "got:\n{out}"
        );
        assert!(out.contains("expect(a, equals(b));"), "got:\n{out}");
        assert!(
            !out.contains("expect(1 + 1, equals(2));"),
            "must not be the vacuous placeholder, got:\n{out}"
        );
        assert!(
            !out.contains("ignore: unused_import"),
            "the import is actually used here, got:\n{out}"
        );
    }

    /// An opaque type has no client-constructible representation, so it can't be literal
    /// -constructed; the seed falls back to a compile-time existence check on the type
    /// name instead of skipping straight to the vacuous placeholder.
    #[test]
    fn scaffold_test_falls_back_to_existence_check_for_an_opaque_type() {
        let api = ApiSurface {
            types: vec![TypeDef {
                name: "Client".to_string(),
                is_opaque: true,
                ..Default::default()
            }],
            ..Default::default()
        };
        let out = scaffold_dart_test(&api, &minimal_config(), "my_lib", "my_lib/my_lib.dart");

        assert!(out.contains("test('my_lib exposes `Client`'"), "got:\n{out}");
        assert!(out.contains("expect(my_lib.Client, isNotNull);"), "got:\n{out}");
        assert!(!out.contains("equals(b)"), "got:\n{out}");
    }

    /// A DTO with an unsupported field shape (e.g. `Optional<T>`) can't be literal
    /// -constructed safely by this seed either — it also falls back to the existence
    /// check rather than emitting a construction call with a guessed value.
    #[test]
    fn scaffold_test_falls_back_to_existence_check_for_unsupported_field_shape() {
        let api = ApiSurface {
            types: vec![TypeDef {
                name: "Config".to_string(),
                has_serde: true,
                fields: vec![FieldDef {
                    name: "nickname".to_string(),
                    ty: TypeRef::Optional(Box::new(TypeRef::String)),
                    optional: true,
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let out = scaffold_dart_test(&api, &minimal_config(), "my_lib", "my_lib/my_lib.dart");

        assert!(out.contains("test('my_lib exposes `Config`'"), "got:\n{out}");
        assert!(out.contains("expect(my_lib.Config, isNotNull);"), "got:\n{out}");
    }

    /// With no visible struct at all, a visible enum is checked for existence instead.
    #[test]
    fn scaffold_test_falls_back_to_existence_check_for_an_enum_when_no_types_exist() {
        let api = ApiSurface {
            enums: vec![EnumDef {
                name: "Color".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let out = scaffold_dart_test(&api, &minimal_config(), "my_lib", "my_lib/my_lib.dart");

        assert!(out.contains("test('my_lib exposes `Color`'"), "got:\n{out}");
        assert!(out.contains("expect(my_lib.Color, isNotNull);"), "got:\n{out}");
    }

    /// `binding_excluded` types were never emitted into the generated Dart module, so the
    /// seed must skip them rather than asserting against a type that doesn't exist.
    #[test]
    fn scaffold_test_skips_binding_excluded_types() {
        let api = ApiSurface {
            types: vec![
                TypeDef {
                    name: "Hidden".to_string(),
                    is_opaque: true,
                    binding_excluded: true,
                    ..Default::default()
                },
                TypeDef {
                    name: "Visible".to_string(),
                    is_opaque: true,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let out = scaffold_dart_test(&api, &minimal_config(), "my_lib", "my_lib/my_lib.dart");

        assert!(out.contains("Visible"), "got:\n{out}");
        assert!(!out.contains("Hidden"), "got:\n{out}");
    }

    /// A genuinely empty API surface (no Rust code written yet) has nothing to assert
    /// against beyond the import resolving — the only honest seed content, and the only
    /// case where the placeholder assertion is legitimate.
    #[test]
    fn scaffold_test_falls_back_to_placeholder_when_api_surface_is_empty() {
        let out = scaffold_dart_test(
            &ApiSurface::default(),
            &minimal_config(),
            "my_lib",
            "my_lib/my_lib.dart",
        );

        assert!(
            out.contains("import 'package:my_lib/my_lib.dart' as my_lib;"),
            "got:\n{out}"
        );
        assert!(out.contains("test('placeholder', () {"), "got:\n{out}");
        assert!(out.contains("expect(1 + 1, equals(2));"), "got:\n{out}");
        assert!(
            out.contains("ignore: unused_import"),
            "placeholder never references the import, got:\n{out}"
        );
    }

    /// End-to-end through `scaffold_dart`: the emitted test file at
    /// `test/{module}_test.dart` must carry a real assertion against the generated API
    /// (not the vacuous `expect(1 + 1, equals(2))` placeholder) whenever the API surface
    /// has something to assert against, and must be `generated_header: false` so the
    /// create-only write-path guard (`write_scaffold_files_report`'s `can_skip`) never
    /// overwrites a real hand-written suite once one exists at that path.
    #[test]
    fn scaffold_dart_emits_real_test_assertions_and_is_create_only() {
        let api = ApiSurface {
            crate_name: "my-lib".to_string(),
            types: vec![simple_type("Widget")],
            ..Default::default()
        };
        let files = scaffold_dart(&api, &minimal_config()).expect("scaffold");
        let test_file = find_file(&files, "packages/dart/test/my_lib_test.dart");

        assert!(
            !test_file.generated_header,
            "test seed must be generated_header: false (create-only)"
        );
        assert!(test_file.content.contains("equals(b)"), "got:\n{}", test_file.content);
        assert!(
            !test_file.content.contains("expect(1 + 1, equals(2));"),
            "must not emit the old vacuous placeholder test, got:\n{}",
            test_file.content
        );
    }

    /// Literal-for-literal reference set used by the migration tests below, all three
    /// verified by directly reading `packages/dart/test/*.dart` in the named repo:
    /// html-to-markdown carries the oldest placeholder shape on disk (no `package:`
    /// import, no rationale comment -- exactly the shape a byte-match against the
    /// *current* generator output would miss); liter-llm and tree-sitter-language-pack
    /// are real hand-written suites that must never be touched.
    fn h2m_historical_placeholder() -> &'static str {
        "import 'package:test/test.dart';\n\nvoid main() {\n  test('placeholder', () {\n    expect(1 + 1, equals(2));\n  });\n}\n"
    }

    fn liter_llm_hand_written_suite() -> &'static str {
        "import 'package:liter_llm/liter_llm.dart';\nimport 'package:test/test.dart';\n\nvoid main() {\n  test('AuthConfig equality is based on authType and envVar', () {\n    const a = AuthConfig(authType: AuthType.bearer, envVar: 'OPENAI_API_KEY');\n    const b = AuthConfig(authType: AuthType.bearer, envVar: 'OPENAI_API_KEY');\n    const c = AuthConfig(authType: AuthType.apiKey, envVar: 'OPENAI_API_KEY');\n\n    expect(a, equals(b));\n    expect(a.hashCode, equals(b.hashCode));\n    expect(a, isNot(equals(c)));\n  });\n}\n"
    }

    /// Verbatim copy of tree-sitter-language-pack's real
    /// `packages/dart/test/tree_sitter_language_pack_test.dart` (162 lines, 17 `expect(`
    /// calls across 18 `test(` blocks) -- the largest, most structurally different of the
    /// three real hand-written suites checked against this predicate: `group()` nesting,
    /// `setUpAll`/`tearDownAll`, async tests, and an unrelated `expectLater(` (which must
    /// not be miscounted as `expect(`, since `expectLater(` does not contain the literal
    /// substring `expect(` followed immediately by `(`... it contains `expect` followed by
    /// `Later(`, so it is correctly excluded by construction, not by a special case).
    fn tree_sitter_language_pack_hand_written_suite() -> &'static str {
        r#"import 'package:test/test.dart';
import 'package:tree_sitter_language_pack/tree_sitter_language_pack.dart';
import 'package:tree_sitter_language_pack/src/tree_sitter_language_pack_bridge_generated/frb_generated.dart' show RustLib;

// Requires the native `tree-sitter-language-pack-dart` Rust crate to be built
// via `task dart:build`, which compiles with TSLP_LINK_MODE=static and
// TSLP_LANGUAGES=mojo,nim,norg (see .task/dart.yml). Every test that needs a
// real grammar (parsing, root-node kind, `process()`) uses "nim", one of
// those three statically-compiled languages, so this suite needs no network
// access and no warm download cache. "python"/"rust"/"markdown" appear only
// as literal data values in the pure language-detection tests below, which
// consult a static extension/shebang lookup table and never touch a parser.

void main() {
  var rustLibInitialized = false;

  setUpAll(() async {
    await RustLib.init();
    rustLibInitialized = true;
  });

  tearDownAll(() async {
    if (rustLibInitialized) {
      RustLib.dispose();
    }
  });

  group('language detection (pure, no grammar required)', () {
    test('should_map_py_extension_to_python', () async {
      final result = await TreeSitterLanguagePackBridge.detectLanguageFromExtension('py');
      expect(result, equals('python'), reason: '"py" is a well-known extension and must resolve to "python"');
    });

    test('should_match_extensions_case_insensitively', () async {
      final result = await TreeSitterLanguagePackBridge.detectLanguageFromExtension('RS');
      expect(result, equals('rust'), reason: 'extension matching must be case-insensitive per documented behavior');
    });

    test('should_return_null_for_unrecognized_extension', () async {
      final result = await TreeSitterLanguagePackBridge.detectLanguageFromExtension(
        'this-extension-does-not-exist-anywhere',
      );
      expect(result, isNull, reason: 'unrecognized extensions must not resolve to any language');
    });

    test('should_detect_rust_from_file_path', () async {
      final result = await TreeSitterLanguagePackBridge.detectLanguageFromPath('src/main.rs');
      expect(result, equals('rust'));
    });

    test('should_return_null_for_path_without_extension', () async {
      final result = await TreeSitterLanguagePackBridge.detectLanguageFromPath('Makefile');
      expect(result, isNull, reason: 'a path with no extension has nothing to detect from');
    });

    test('should_detect_python_from_env_shebang', () async {
      final result = await TreeSitterLanguagePackBridge.detectLanguageFromContent('#!/usr/bin/env python3\npass');
      expect(result, equals('python'));
    });

    test('should_return_null_when_content_has_no_shebang', () async {
      final result = await TreeSitterLanguagePackBridge.detectLanguageFromContent('no shebang here');
      expect(result, isNull);
    });

    test('should_alias_detect_language_to_path_extension_lookup', () async {
      final result = await TreeSitterLanguagePackBridge.detectLanguage('README.md');
      expect(result, equals('markdown'), reason: 'detectLanguage is documented as a path/extension detection alias');
    });
  });

  group('bundled queries (pure, no grammar required)', () {
    test('should_return_null_highlights_query_for_unknown_language', () async {
      final result = await TreeSitterLanguagePackBridge.getHighlightsQuery('this-language-does-not-exist');
      expect(result, isNull);
    });
  });

  group('registry (statically compiled "nim", no network required)', () {
    test('should_report_statically_compiled_language_as_available', () async {
      final result = await TreeSitterLanguagePackBridge.hasLanguage('nim');
      expect(result, isTrue, reason: 'nim is compiled in by task dart:build (TSLP_LANGUAGES=mojo,nim,norg)');
    });

    test('should_report_unknown_language_as_unavailable', () async {
      final result = await TreeSitterLanguagePackBridge.hasLanguage('totally-bogus-language-name');
      expect(result, isFalse);
    });

    test('should_list_statically_compiled_language_in_available_languages', () async {
      final result = await TreeSitterLanguagePackBridge.availableLanguages();
      expect(result, contains('nim'));
    });

    test('should_keep_language_count_consistent_with_available_languages_list', () async {
      final count = await TreeSitterLanguagePackBridge.languageCount();
      final names = await TreeSitterLanguagePackBridge.availableLanguages();
      expect(
        count,
        equals(names.length),
        reason: 'languageCount() must always equal availableLanguages().length; a mismatch means one of the two '
            'accessors is stale relative to the other',
      );
    });
  });

  group('parsing (statically compiled "nim", no network required)', () {
    // parsers/nim/src/grammar.json names its start rule "module", and
    // node-types.json confirms "module" is a named node type — verified
    // directly against the grammar sources vendored in this repo, not
    // assumed from another language's doc example.
    test('should_parse_nim_source_to_a_module_root_node', () async {
      final parser = await TreeSitterLanguagePackBridge.getParser('nim');
      final tree = await parser.parse(source: 'echo "hello"');
      expect(tree, isNotNull, reason: 'parsing valid nim source must produce a tree');
      final root = await tree!.rootNode();
      final kind = await root.kind();
      expect(kind, equals('module'), reason: 'nim\'s tree-sitter grammar names its root node "module"');
    });

    // fixtures/smoke/nim.json asserts `not_error` for this exact source, so
    // it is known-valid nim, not a guess.
    test('should_report_zero_errors_for_syntactically_valid_nim', () async {
      final config = await createProcessConfigFromJson(json: '{"language":"nim"}');
      final result = await TreeSitterLanguagePackBridge.process('echo "hello"', config: config);
      expect(result.language, equals('nim'));
      expect(result.metrics.errorCount, equals(0));
    });

    // No structure-extraction test: crates/ts-pack-core/src/intel/intelligence.rs
    // `structure_kind_at()` matches an exact, hardcoded set of tree-sitter node
    // kind names (`function_definition`, `function_item`, `struct_item`, ...),
    // and nim's grammar (parsers/nim/src/node-types.json) uses none of them —
    // its declarations are named `declaration` / `declColonEquals`. Structure
    // extraction is therefore unimplemented for nim and would always report an
    // empty list; asserting that would be vacuous, so the test is omitted
    // rather than weakened to a truthiness check.
  });

  group('error paths', () {
    test('should_throw_when_getting_an_unknown_language', () async {
      await expectLater(
        TreeSitterLanguagePackBridge.getLanguage('this-language-does-not-exist-anywhere'),
        throwsA(anything),
      );
    });

    test('should_throw_when_getting_a_parser_for_an_unknown_language', () async {
      await expectLater(
        TreeSitterLanguagePackBridge.getParser('this-language-does-not-exist-anywhere'),
        throwsA(anything),
      );
    });

    test('should_throw_when_processing_with_an_empty_language_name', () async {
      await expectLater(() async {
        final config = await createProcessConfigFromJson(json: '{"language":""}');
        return TreeSitterLanguagePackBridge.process('hello', config: config);
      }(), throwsA(anything));
    });
  });
}
"#
    }

    /// `is_vacuous_dart_placeholder` fires for html-to-markdown's real on-disk shape
    /// (1 `expect(`, 1 `test(`) and refuses liter-llm's and tree-sitter-language-pack's
    /// real hand-written suites (3/1 and 17/18 respectively) -- pinning the counts this
    /// predicate depends on directly against the trees the migration exists to repair,
    /// not just against synthetic fixtures.
    #[test]
    fn vacuity_signature_matches_real_consumer_trees() {
        assert!(is_vacuous_dart_placeholder(h2m_historical_placeholder()));
        assert!(!is_vacuous_dart_placeholder(liter_llm_hand_written_suite()));
        assert!(!is_vacuous_dart_placeholder(
            tree_sitter_language_pack_hand_written_suite()
        ));
    }

    /// tree-sitter-language-pack's real 162-line hand-written suite, the largest and
    /// structurally most different of the three real trees checked, must never be
    /// reported as a migration candidate.
    #[test]
    fn does_not_touch_the_tree_sitter_language_pack_hand_written_suite() {
        let dir = tempfile::tempdir().expect("tempdir");
        let test_dir = dir.path().join("packages/dart/test");
        std::fs::create_dir_all(&test_dir).expect("create packages/dart/test");
        let hand_written = tree_sitter_language_pack_hand_written_suite();
        std::fs::write(test_dir.join("tree_sitter_language_pack_test.dart"), hand_written)
            .expect("write hand-written test");

        let relative_path = std::path::Path::new("packages/dart/test/tree_sitter_language_pack_test.dart");
        let changed = migrate_dart_placeholder_test(dir.path(), relative_path, "anything else entirely")
            .expect("migration must not error");
        assert!(!changed, "a hand-written test must never be reported as changed");

        let on_disk = std::fs::read_to_string(test_dir.join("tree_sitter_language_pack_test.dart")).expect("read file");
        assert_eq!(on_disk, hand_written, "hand-written test must survive byte-for-byte");
    }

    /// Requirement 1: the *current* generator's own placeholder output -- regenerated
    /// here, never hardcoded, since hardcoding this exact string is how the byte-match
    /// version of this migration silently stopped matching its own generator's output
    /// after the template grew a rationale comment -- still migrates.
    #[test]
    fn migrates_the_current_generators_placeholder_output() {
        let dir = tempfile::tempdir().expect("tempdir");
        let test_dir = dir.path().join("packages/dart/test");
        std::fs::create_dir_all(&test_dir).expect("create packages/dart/test");
        let current_placeholder = dart_placeholder_test("my_lib", "my_lib/my_lib.dart");
        std::fs::write(test_dir.join("my_lib_test.dart"), &current_placeholder).expect("write current placeholder");

        let replacement = dart_equality_round_trip_test(
            "my_lib",
            "my_lib/my_lib.dart",
            "Widget",
            &[SimpleDartField {
                label: "count".to_string(),
                literal: "1".to_string(),
            }],
        );
        let relative_path = std::path::Path::new("packages/dart/test/my_lib_test.dart");
        let changed =
            migrate_dart_placeholder_test(dir.path(), relative_path, &replacement).expect("migration must not error");
        assert!(
            changed,
            "the current generator's own placeholder must be reported as changed"
        );

        let on_disk = std::fs::read_to_string(test_dir.join("my_lib_test.dart")).expect("read migrated file");
        assert_eq!(on_disk, replacement);
    }

    /// Requirement 2: the exact marker-less, import-less shape found on disk in
    /// html-to-markdown -- the historical placeholder revision this migration exists to
    /// reach, and the one a byte-match against only the current template would miss --
    /// migrates, and a second pass over the result is a no-op (requirement 4).
    #[test]
    fn migrates_the_h2m_historical_shape_and_is_idempotent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let test_dir = dir.path().join("packages/dart/test");
        std::fs::create_dir_all(&test_dir).expect("create packages/dart/test");
        std::fs::write(
            test_dir.join("html_to_markdown_rs_test.dart"),
            h2m_historical_placeholder(),
        )
        .expect("write h2m historical placeholder");

        let replacement = dart_equality_round_trip_test(
            "html_to_markdown_rs",
            "html_to_markdown_rs/html_to_markdown_rs.dart",
            "Widget",
            &[SimpleDartField {
                label: "count".to_string(),
                literal: "1".to_string(),
            }],
        );
        let relative_path = std::path::Path::new("packages/dart/test/html_to_markdown_rs_test.dart");
        let changed =
            migrate_dart_placeholder_test(dir.path(), relative_path, &replacement).expect("migration must not error");
        assert!(
            changed,
            "h2m's real historical placeholder shape must be reported as changed"
        );

        let on_disk =
            std::fs::read_to_string(test_dir.join("html_to_markdown_rs_test.dart")).expect("read migrated file");
        assert_eq!(on_disk, replacement);

        let changed_again =
            migrate_dart_placeholder_test(dir.path(), relative_path, &replacement).expect("second pass must not error");
        assert!(
            !changed_again,
            "second pass over an already-migrated file must be a no-op"
        );
    }

    /// Requirement 3: liter-llm's real hand-written suite (3 `expect(` calls, a
    /// descriptive test name, no `'placeholder'` anywhere) must never be touched, byte
    /// for byte -- the same guarantee `migrate_build_zig_test_target` provides for
    /// `build.zig`.
    #[test]
    fn does_not_touch_the_liter_llm_hand_written_suite() {
        let dir = tempfile::tempdir().expect("tempdir");
        let test_dir = dir.path().join("packages/dart/test");
        std::fs::create_dir_all(&test_dir).expect("create packages/dart/test");
        let hand_written = liter_llm_hand_written_suite();
        std::fs::write(test_dir.join("liter_llm_test.dart"), hand_written).expect("write hand-written test");

        let relative_path = std::path::Path::new("packages/dart/test/liter_llm_test.dart");
        let changed = migrate_dart_placeholder_test(dir.path(), relative_path, "anything else entirely")
            .expect("migration must not error");
        assert!(!changed, "a hand-written test must never be reported as changed");

        let on_disk = std::fs::read_to_string(test_dir.join("liter_llm_test.dart")).expect("read file");
        assert_eq!(on_disk, hand_written, "hand-written test must survive byte-for-byte");
    }

    /// Requirement 5: a `test/*_test.dart` that does not exist yet (nothing scaffolded so
    /// far) must not be created or error -- there is nothing to migrate.
    #[test]
    fn migrate_dart_placeholder_is_a_no_op_when_file_does_not_exist() {
        let dir = tempfile::tempdir().expect("tempdir");
        let relative_path = std::path::Path::new("packages/dart/test/my_lib_test.dart");
        let changed = migrate_dart_placeholder_test(dir.path(), relative_path, "new content").expect("must not error");
        assert!(!changed);
        assert!(!dir.path().join(relative_path).exists());
    }

    #[test]
    fn should_replace_stale_pubignore_that_excludes_native_libraries() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pkg_dir = dir.path().join("packages/dart");
        std::fs::create_dir_all(&pkg_dir).expect("create packages/dart");
        std::fs::write(pkg_dir.join(".pubignore"), STALE_PUBIGNORE).expect("write stale .pubignore");

        let replacement = "android/\nios/\nblobs/\nrust/\nexample/\ntest/\n";
        let relative_path = std::path::Path::new("packages/dart/.pubignore");
        let changed = migrate_dart_pubignore(dir.path(), relative_path, replacement).expect("migration must not error");
        assert!(changed, "the known-stale .pubignore must be reported as changed");

        let on_disk = std::fs::read_to_string(pkg_dir.join(".pubignore")).expect("read migrated file");
        assert_eq!(on_disk, replacement);
        assert!(
            !on_disk.contains("lib/src/native/") && !on_disk.contains("*.so"),
            "native FFI libraries must no longer be excluded"
        );

        let changed_again =
            migrate_dart_pubignore(dir.path(), relative_path, replacement).expect("second pass must not error");
        assert!(
            !changed_again,
            "second pass over an already-migrated file must be a no-op"
        );
    }

    #[test]
    fn should_not_touch_a_hand_edited_pubignore() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pkg_dir = dir.path().join("packages/dart");
        std::fs::create_dir_all(&pkg_dir).expect("create packages/dart");
        let hand_written =
            "android/\nios/\nblobs/\nlib/src/native/\nrust/\nexample/\ntest/\n*.so\n*.dylib\n*.dll\ncustom_ignore/\n";
        std::fs::write(pkg_dir.join(".pubignore"), hand_written).expect("write hand-edited .pubignore");

        let relative_path = std::path::Path::new("packages/dart/.pubignore");
        let changed = migrate_dart_pubignore(
            dir.path(),
            relative_path,
            "android/\nios/\nblobs/\nrust/\nexample/\ntest/\n",
        )
        .expect("migration must not error");
        assert!(!changed, "a .pubignore with any consumer edit must never be touched");

        let on_disk = std::fs::read_to_string(pkg_dir.join(".pubignore")).expect("read file");
        assert_eq!(
            on_disk, hand_written,
            "hand-edited .pubignore must survive byte-for-byte"
        );
    }

    #[test]
    fn migrate_dart_pubignore_is_a_no_op_when_file_does_not_exist() {
        let dir = tempfile::tempdir().expect("tempdir");
        let relative_path = std::path::Path::new("packages/dart/.pubignore");
        let changed = migrate_dart_pubignore(dir.path(), relative_path, "new content").expect("must not error");
        assert!(!changed);
        assert!(!dir.path().join(relative_path).exists());
    }
}
