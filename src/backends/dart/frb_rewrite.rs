//! Post-processing rewriter for flutter_rust_bridge-generated `lib.dart`.
//!
//! flutter_rust_bridge emits sealed-class tagged-union variants with positional
//! parameter names (`field0`, `field1`, ...) when the underlying Rust variant
//! is a tuple variant:
//!
//! ```dart
//! const factory FormatMetadata.pdf({required PdfMetadata field0}) =
//!     FormatMetadata_Pdf;
//! ```
//!
//! These positional names are awkward for callers and inconsistent with the
//! kotlin/swift/etc. binding surface, which derives payload-informed names
//! (`metadata`, `value`, `value0`, ...) using the shared algorithm defined in
//! `alef-backend-kotlin::gen_bindings::shared::kotlin_field_name_with_type`.
//!
//! [`rewrite_frb_sealed_variants`] post-processes the frb-generated source and
//! rewrites variant parameter names to match the payload-derived convention.
//! Other code in the file is left untouched.
//!
//! Algorithm (per variant declaration line(s)):
//! 1. Match the canonical frb sealed-variant signature:
//!    `const factory <Enum>.<variantCamel>({required <PayloadType> field<N>, ...}) = <Enum>_<VariantPascal>;`
//! 2. Recover the `VariantPascal` token from the trailing assignment so that
//!    the variant name is unambiguous (the dotted form is lowerCamel, which
//!    cannot be reliably inverted back to PascalCase for multi-word variants).
//! 3. For each `field<N>` parameter, derive its new name from the payload type
//!    using the payload-derived helper (see [`payload_param_name`]).

use regex::Regex;
use std::sync::OnceLock;

/// Idempotency marker injected into `RustLib.init` by
/// [`rewrite_frb_external_library_loader`]. Presence of this token means the
/// loader override has already been applied, so the rewrite is a no-op.
const ALEF_LOADER_MARKER: &str = "_alefResolveExternalLibrary";

/// Inject a published-package-aware native-library loader into the
/// flutter_rust_bridge-generated `frb_generated.dart`.
///
/// # Why
///
/// flutter_rust_bridge's default loader (`kDefaultExternalLibraryLoaderConfig`)
/// uses a build-tree-relative `ioDirectory` (e.g. `rust/target/release/`) that
/// is resolved against the *consumer's* current working directory and is NOT
/// shipped in the published pub tarball. When the package is consumed from
/// pub.dev the default loader fails to find the library at that path and falls
/// back to opening a relative framework path (`<stem>.framework/<stem>` on
/// macOS), which a hardened runtime rejects with
/// "Failed to load dynamic library ... (relative path not allowed)".
///
/// # Fix
///
/// This rewrite makes `RustLib.init` resolve the prebuilt native library from
/// the package's *own* installed location (`lib/src/<module>_bridge_generated/`,
/// resolved at runtime via `Isolate.resolvePackageUri`) as an **absolute** path
/// before delegating to flutter_rust_bridge. The publish pipeline ships the
/// prebuilt library alongside the generated bridge sources there. When the
/// package-relative library cannot be found (e.g. local development where the
/// library lives under `rust/target/<profile>/`), the override returns `null`
/// and flutter_rust_bridge falls back to its default loader unchanged — so this
/// is safe in both published and source-tree builds.
///
/// The transform is **idempotent**: a source that already contains the injected
/// helper is returned verbatim. It is also a no-op on any source that does not
/// contain the canonical FRB `RustLib.init` prologue (e.g. `lib.dart`), so it is
/// safe to apply unconditionally to any frb-generated file.
///
/// `package_name` is the pub package name (used to build the `package:` URI),
/// `module_name` is the bridge module stem (the `<module>_bridge_generated`
/// directory), and `stem` is the native library file stem
/// (`kDefaultExternalLibraryLoaderConfig.stem`, e.g. `sample_project_dart`).
pub fn rewrite_frb_external_library_loader(source: &str, package_name: &str, module_name: &str, stem: &str) -> String {
    if source.contains(ALEF_LOADER_MARKER) {
        return source.to_string();
    }
    let Some(prologue) = frb_init_prologue(source) else {
        return source.to_string();
    };

    let replacement = frb_init_prologue_replacement(package_name, module_name, stem);
    let with_loader = source.replacen(&prologue, &replacement, 1);

    ensure_loader_imports(&with_loader)
}

/// Return the exact FRB-generated `RustLib.init` prologue present in `source`,
/// up to and including the `async {` that opens the method body, or `None` if
/// the canonical signature is absent.
///
/// Matches the prologue with flexible indentation, since flutter_rust_bridge
/// emits different indentation in different versions.
fn frb_init_prologue(source: &str) -> Option<String> {
    let re = init_prologue_regex();
    re.find(source).map(|m| m.as_str().to_string())
}

fn init_prologue_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // Match the init prologue with flexible whitespace indentation and parameter order.
        // FRB generates: `static Future<void> init({ ... }) async {`
        // We match from "Initialize flutter_rust_bridge" comment through the opening brace.
        Regex::new(r"(?m)^\s*/// Initialize flutter_rust_bridge\n\s*static Future<void> init\((?s:.)*?\}\) async \{\n")
            .expect("init prologue regex must compile")
    })
}

/// Build the patched `RustLib.init` prologue: the original signature plus a
/// `externalLibrary ??= ...` resolution line, followed by the
/// `_alefResolveExternalLibrary` helper method.
///
/// # Brace Balancing
/// The generated string maintains balanced braces and parentheses. The
/// `_alefResolveExternalLibrary()` helper method is fully closed (lines 126–204
/// in the template), and the `init()` method signature and start are opened
/// (lines 207–213), allowing the original FRB method body to follow seamlessly.
fn frb_init_prologue_replacement(package_name: &str, module_name: &str, stem: &str) -> String {
    format!(
        r#"  /// Resolve the prebuilt native library from environment variable,
  /// package-relative location, or defer to flutter_rust_bridge's default loader.
  /// Returns `null` to defer to flutter_rust_bridge's default loader.
  ///
  /// Checks in order:
  /// 1. FRB_DART_LOAD_EXTERNAL_LIBRARY_NATIVE_LIB_DIR environment variable
  ///    (allows test harnesses to point to development build paths)
  /// 2. Package-installed location with RID subdirectory (lib/src/native/<rid>/)
  ///    (for published pub.dev packages with platform-specific bundled native libraries)
  /// 3. Package-installed location (lib/src/{module}_bridge_generated/)
  ///    (legacy fallback for development or packages without per-platform binaries)
  /// 4. Returns null (flutter_rust_bridge falls back to its default loader)
  static Future<ExternalLibrary?> {marker}() async {{
    try {{
      const candidates = <String>[
        'lib{stem}.dylib',
        'lib{stem}.so',
        '{stem}.dll',
      ];

      // Check FRB_DART_LOAD_EXTERNAL_LIBRARY_NATIVE_LIB_DIR env var first.
      // This allows test harnesses to override library location for development.
      final envDir = Platform.environment['FRB_DART_LOAD_EXTERNAL_LIBRARY_NATIVE_LIB_DIR'];
      if (envDir != null && envDir.isNotEmpty) {{
        final libDir = Directory(envDir);
        if (libDir.existsSync()) {{
          for (final candidate in candidates) {{
            final libPath = '$envDir/$candidate';
            if (File(libPath).existsSync()) {{
              return ExternalLibrary.open(libPath);
            }}
          }}
        }}
      }}

      // Compute RID (runtime identifier) from platform and architecture.
      String? computeRid() {{
        final os = Platform.operatingSystem;
        // Use Dart's Platform.version to detect architecture.
        // Format: "Dart <version> (stable) ... on \"<os> <arch>\""
        final version = Platform.version;
        final archMatch = version.contains('x86_64') ? 'x64'
            : version.contains('aarch64') || version.contains('arm64') ? 'arm64'
            : version.contains('armv7') ? 'arm'
            : null;
        if (archMatch == null) return null;

        switch (os) {{
          case 'linux':
            return 'linux-$archMatch';
          case 'macos':
            return 'macos-$archMatch';
          case 'windows':
            return 'windows-$archMatch';
          default:
            return null;
        }}
      }}

      final rid = computeRid();
      if (rid != null) {{
        final packageRoot =
            await Isolate.resolvePackageUri(Uri.parse('package:{package}/{package}.dart'));
        if (packageRoot != null) {{
          final ridDir = packageRoot.resolve('src/native/$rid/');
          for (final candidate in candidates) {{
            final libPath = ridDir.resolve(candidate).toFilePath();
            if (File(libPath).existsSync()) {{
              return ExternalLibrary.open(libPath);
            }}
          }}
        }}
      }}

      // Check legacy package-installed location as fallback.
      final packageRoot =
          await Isolate.resolvePackageUri(Uri.parse('package:{package}/{package}.dart'));
      if (packageRoot != null) {{
        final libDir = packageRoot.resolve('src/{module}_bridge_generated/');
        for (final candidate in candidates) {{
          final libPath = libDir.resolve(candidate).toFilePath();
          if (File(libPath).existsSync()) {{
            return ExternalLibrary.open(libPath);
          }}
        }}
      }}
    }} catch (_) {{
      // Fall through to the default loader on any resolution failure.
    }}
    return null;
  }}

  /// Initialize flutter_rust_bridge
  static Future<void> init({{
    RustLibApi? api,
    BaseHandler? handler,
    ExternalLibrary? externalLibrary,
    bool forceSameCodegenVersion = true,
  }}) async {{
    externalLibrary ??= await {marker}();
"#,
        marker = ALEF_LOADER_MARKER,
        package = package_name,
        module = module_name,
        stem = stem,
    )
}

/// Ensure `dart:io`, `dart:isolate`, and `dart:core` are imported (the loader helper uses
/// `File`, `Isolate`, and `Uri`). Inserts the imports after the first existing `import`
/// line if missing. Idempotent.
///
/// To avoid namespace conflict with the FRB-generated `Uri` class, imports
/// `dart:core.Uri` with an alias (`_DartCoreUri`), then replaces all
/// `Uri.parse()` and `Uri.resolve()` calls with the aliased name.
fn ensure_loader_imports(source: &str) -> String {
    let mut result = source.to_string();
    let needed = [
        ("import 'dart:core' as _DartCore;", "import 'dart:core' as _DartCore;\n"),
        ("import 'dart:io';", "import 'dart:io';\n"),
        ("import 'dart:isolate';", "import 'dart:isolate';\n"),
    ];

    // Find the first import line to anchor insertions so the added imports sit
    // alongside the existing import block.
    let anchor = result.find("\nimport ").map(|i| i + 1);
    for (probe, line) in needed {
        if result.contains(probe) {
            continue;
        }
        match anchor {
            Some(pos) => result.insert_str(pos, line),
            None => result.insert_str(0, line),
        }
    }

    // Replace Uri.parse() with qualified name to avoid conflict with the generated Uri class.
    // Note: .resolve() is called on Uri instances, so it doesn't need qualification.
    result = result.replace("Uri.parse(", "_DartCore.Uri.parse(");

    result
}

/// Extract the native-library stem from the FRB-generated
/// `kDefaultExternalLibraryLoaderConfig` (the `stem: '<name>'` field), or `None`
/// if the config block is absent (e.g. for `lib.dart`).
fn extract_loader_stem(source: &str) -> Option<String> {
    let re = stem_regex();
    re.captures(source).map(|c| c["stem"].to_string())
}

fn stem_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"stem:\s*'(?P<stem>[A-Za-z0-9_]+)'").expect("stem regex must compile"))
}

/// Apply the published-package loader fix to a frb-generated file, deriving the
/// package, bridge-module, and library stem from the file's own
/// `kDefaultExternalLibraryLoaderConfig`.
///
/// alef's dart backend names the bridge cdylib `<crate>_dart` (the FRB `stem`),
/// emits its bridge sources under `lib/src/<crate>_bridge_generated/`, and (by
/// default) publishes the package as `<crate>`. The shared `<crate>` prefix is
/// recovered by stripping the trailing `_dart` from the stem, which is the
/// information needed to resolve the package's own native library at runtime.
///
/// No-op when no loader config is present (returns `source` unchanged), so this
/// is safe to call on `lib.dart` as well as `frb_generated.dart`.
fn apply_loader_fix_from_stem(source: &str) -> String {
    let Some(stem) = extract_loader_stem(source) else {
        return source.to_string();
    };
    // Recover the shared crate name from `<crate>_dart`; if the stem does not
    // follow the convention, fall back to the full stem for both package and
    // module so the resolution at least targets a plausible path.
    let crate_base = stem.strip_suffix("_dart").unwrap_or(&stem);
    let package_name = crate_base;
    let module_name = crate_base;
    rewrite_frb_external_library_loader(source, package_name, module_name, &stem)
}

/// Rewrite all flutter_rust_bridge sealed-class variant parameter names in
/// `source` from positional (`field0`, `field1`, ...) to payload-derived names.
///
/// Returns the rewritten source. Lines that do not match the variant signature
/// are returned verbatim, so this function is safe to apply unconditionally to
/// any frb-generated `lib.dart`.
///
/// When applied to `frb_generated.dart` (which carries the FRB external-library
/// loader config) this also injects the published-package native-library loader
/// via [`rewrite_frb_external_library_loader`]; the injection is idempotent and a
/// no-op for files without the loader config.
pub fn rewrite_frb_sealed_variants(source: &str) -> String {
    let source = apply_loader_fix_from_stem(source);
    let source = source.as_str();
    let variant_re = variant_regex();

    variant_re
        .replace_all(source, |caps: &regex::Captures<'_>| {
            let prefix = &caps["prefix"];
            let params = &caps["params"];
            let suffix = &caps["suffix"];
            let variant_pascal = &caps["variant"];

            let rewritten_params = rewrite_param_list(params, variant_pascal);
            format!("{prefix}{rewritten_params}{suffix}")
        })
        .into_owned()
}

/// Match a single frb sealed-variant `const factory` line, allowing the
/// parameter block and the trailing `= <Enum>_<Variant>;` to span lines.
///
/// Capture groups:
/// - `prefix`  — `const factory <Enum>.<variantCamel>({`
/// - `params`  — the parameter list between `{` and `}` (no braces)
/// - `suffix`  — `}) = <Enum>_<Variant>;`
/// - `variant` — `<Variant>` (PascalCase) extracted from the assignment
fn variant_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // The frb output can wrap before the `=` or before the `_Variant`
        // identifier. Use `(?s)` so `.` matches newlines inside the param block
        // and the inter-token whitespace.
        Regex::new(
            r"(?s)(?P<prefix>const\s+factory\s+[A-Za-z_][A-Za-z0-9_]*\.[A-Za-z_][A-Za-z0-9_]*\s*\(\s*\{)(?P<params>[^{}]*)(?P<suffix>\}\s*\)\s*=\s*[A-Za-z_][A-Za-z0-9_]*_(?P<variant>[A-Za-z][A-Za-z0-9]*)\s*;)",
        )
        .expect("variant regex must compile")
    })
}

/// Rewrite the comma-separated parameter list inside the variant constructor.
///
/// Each parameter has the shape `required <Type> field<N>`. The `<Type>` can be
/// `List<T>`, `Map<K, V>`, nullable types, etc. — we capture everything between
/// the leading `required` keyword and the trailing positional `field<N>` ident.
fn rewrite_param_list(params: &str, variant_pascal: &str) -> String {
    let param_re = param_regex();

    // Collect each `required <type> field<N>` match so we know `total_fields`
    // before deriving names (the payload-derived algorithm differs for
    // single-field vs multi-field variants).
    let matches: Vec<regex::Captures<'_>> = param_re.captures_iter(params).collect();
    let total_fields = matches
        .iter()
        .filter(|m| {
            let name = m.name("name").map(|m| m.as_str()).unwrap_or("");
            is_positional_field(name)
        })
        .count();

    if total_fields == 0 {
        return params.to_string();
    }

    let mut out = String::with_capacity(params.len());
    let mut cursor = 0usize;

    for caps in &matches {
        let whole = caps.get(0).expect("regex match must have group 0");
        let name_match = caps.name("name").expect("name capture is required");
        let raw_name = name_match.as_str();

        // Preserve everything between the previous match end and the start of
        // this `field<N>` ident untouched (whitespace, commas, the `required`
        // keyword, and the type).
        out.push_str(&params[cursor..name_match.start()]);

        if let Some(field_idx) = field_index(raw_name) {
            let type_name = caps.name("type").map(|m| m.as_str()).unwrap_or("").trim();
            let new_name = payload_param_name(type_name, variant_pascal, field_idx, total_fields);
            out.push_str(&new_name);
        } else {
            // Already a named parameter; leave it alone.
            out.push_str(raw_name);
        }

        cursor = name_match.end();
        // Continue from the end of the ident; the rest of `whole` (if any) is
        // accounted for by the next iteration's prefix slice.
        let _ = whole; // silence unused warning when no debug
    }

    out.push_str(&params[cursor..]);
    out
}

/// Match a single `required <Type> <ident>` parameter inside a frb variant
/// constructor. The `<Type>` is a balanced-bracket-free chunk plus `?` for
/// nullable; this is sufficient because frb does not emit nested generic types
/// like `Map<List<T>, U>` for sealed variants — but to be defensive we accept
/// any characters except `{`, `}`, `,` between the keyword and the ident.
fn param_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"required\s+(?P<type>[^,{}]+?)\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*(?:,|$)")
            .expect("param regex must compile")
    })
}

/// Return the numeric index `N` for a positional `field<N>` parameter, or
/// `None` if `name` is a named (non-positional) parameter.
fn field_index(name: &str) -> Option<usize> {
    let rest = name.strip_prefix("field")?;
    rest.parse::<usize>().ok()
}

/// `true` when `name` is a positional `field<N>` parameter emitted by frb.
fn is_positional_field(name: &str) -> bool {
    field_index(name).is_some()
}

/// Derive a payload-informed parameter name for a single frb variant field.
///
/// Mirrors the algorithm in `alef-backend-kotlin::gen_bindings::shared::
/// kotlin_field_name_with_type` so that all "modern" backends (kotlin, swift,
/// dart, ...) converge on the same surface.
///
/// Rules:
/// - Single-field variant with a named struct payload whose name starts with
///   the variant name (`PdfMetadata` for variant `Pdf`) → strip the prefix and
///   lowerCamelCase the remainder (`Metadata` → `metadata`).
/// - Single-field variant with a primitive/stdlib payload (`String`, `int`,
///   etc.) → `value`.
/// - Multi-field variant → `value0`, `value1`, ... (uses `field_idx`).
/// - Otherwise (single-field with no inferable prefix) → `value`.
fn payload_param_name(type_name: &str, variant_pascal: &str, field_idx: usize, total_fields: usize) -> String {
    if total_fields > 1 {
        return format!("value{field_idx}");
    }

    // Strip a trailing `?` (nullable marker) and any generic args for the
    // prefix-matching check — but keep them out of the primitive lookup as
    // well, since `List<int>` should not be considered "primitive".
    let stripped_type = type_name.trim_end_matches('?');
    let base_type = stripped_type
        .split_once('<')
        .map(|(head, _)| head)
        .unwrap_or(stripped_type)
        .trim();

    if let Some(remainder) = base_type.strip_prefix(variant_pascal)
        && !remainder.is_empty()
    {
        return to_lower_camel(remainder);
    }

    if is_dart_primitive(base_type) {
        return "value".to_string();
    }

    "value".to_string()
}

/// Lower the first character of a PascalCase identifier; preserve the rest.
fn to_lower_camel(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Dart primitive / stdlib types that should map to a generic `value` name.
fn is_dart_primitive(type_name: &str) -> bool {
    matches!(
        type_name,
        "String"
            | "int"
            | "double"
            | "bool"
            | "num"
            | "void"
            | "dynamic"
            | "Object"
            | "Uint8List"
            | "List"
            | "Map"
            | "Set"
            | "BigInt"
            | "DateTime"
            | "Duration"
    )
}

/// Filter out function definitions for excluded function names from FRB-generated `lib.dart`.
///
/// FRB generates public `Future<T> functionName(...)` wrappers for all public functions
/// in the Rust API. This function removes lines that define functions whose names match
/// the provided exclude set, allowing the Dart wrapper class to honor `exclude_functions`
/// config without re-parsing the FRB output.
///
/// The function is idempotent: running it multiple times produces the same result.
pub fn filter_excluded_functions(source: &str, exclude_functions: &std::collections::HashSet<&str>) -> String {
    if exclude_functions.is_empty() {
        return source.to_string();
    }

    let lines: Vec<&str> = source.lines().collect();
    let mut result = String::with_capacity(source.len());
    let mut i = 0;
    let mut doc_buffer: Vec<&str> = Vec::new(); // Buffer for doc comments

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();

        // Check if this is a doc/comment line
        if trimmed.starts_with("///")
            || trimmed.starts_with("//")
            || (trimmed.starts_with("*") && !trimmed.starts_with("**/"))
        {
            // Buffer the comment line
            doc_buffer.push(line);
            i += 1;
            continue;
        }

        // Check if this is the start of a function definition we should exclude.
        // Match function signature lines that contain a function name followed by `(`
        let mut should_skip_function = false;
        if !trimmed.is_empty() && !trimmed.starts_with("class") && !trimmed.starts_with("enum") {
            should_skip_function = exclude_functions.iter().any(|&excluded| {
                // Convert snake_case to lowerCamelCase to match Dart's function naming
                let camel_excluded = snake_to_camel(excluded);

                // Match patterns like:
                // - `Future<double> functionName({`
                // - `void functionName(`
                // - `String functionName(`
                let pattern = format!(" {}(", camel_excluded);
                line.contains(&pattern)
            });
        }

        if should_skip_function {
            // Clear the buffered doc comments since we're skipping this function
            doc_buffer.clear();
            // Skip this line and all continuation lines until we find a line ending with `;`
            loop {
                i += 1;
                if i >= lines.len() {
                    break;
                }
                let check_line = lines[i];
                if check_line.contains(';') {
                    i += 1;
                    break;
                }
            }
        } else {
            // Keep all buffered doc comments and this line
            for doc_line in &doc_buffer {
                result.push_str(doc_line);
                result.push('\n');
            }
            doc_buffer.clear();
            result.push_str(line);
            result.push('\n');
            i += 1;
        }
    }

    // Append any remaining buffered comments (shouldn't happen, but be safe)
    for doc_line in &doc_buffer {
        result.push_str(doc_line);
        result.push('\n');
    }

    result
}

/// Convert Rust snake_case to Dart lowerCamelCase
fn snake_to_camel(name: &str) -> String {
    let mut result = String::new();
    let mut capitalize_next = false;

    for c in name.chars() {
        if c == '_' {
            capitalize_next = true;
        } else if capitalize_next {
            for upper_c in c.to_uppercase() {
                result.push(upper_c);
            }
            capitalize_next = false;
        } else if result.is_empty() {
            for lower_c in c.to_lowercase() {
                result.push(lower_c);
            }
        } else {
            result.push(c);
        }
    }

    result
}

/// Make struct constructor fields optional for types with Rust defaults.
///
/// Processes struct definitions to make non-optional fields optional in the
/// Dart constructor when the Rust struct has `#[serde(default)]` or similar
/// attributes indicating a default value.
///
/// Currently targets specific sample_crate types that are known to have defaults:
/// - `EmbeddingConfig`: model, normalize, batchSize, showDownloadProgress, acceleration, maxEmbedDurationSecs
/// - `ChunkingConfig`: similar pattern
/// - `ExtractionConfig`: similar pattern (though most are already optional)
///
/// The function rewrites constructor declarations from:
/// ```dart
/// const EmbeddingConfig({
///   required this.model,
///   required this.normalize,
///   ...
/// });
/// ```
///
/// To:
/// ```dart
/// const EmbeddingConfig({
///   this.model,
///   this.normalize,
///   ...
/// });
/// ```
pub fn make_struct_fields_with_defaults_optional(source: &str) -> String {
    // Map of struct names to field names that should be made optional.
    // These correspond to Rust fields with #[serde(default...)] or similar.
    let optional_fields: std::collections::HashMap<&str, Vec<&str>> = [
        (
            "EmbeddingConfig",
            vec![
                "model",
                "normalize",
                "batchSize",
                "showDownloadProgress",
                "acceleration",
                "maxEmbedDurationSecs",
            ],
        ),
        (
            "ChunkingConfig",
            vec![
                "maxCharacters",
                "overlap",
                "trim",
                "chunkerType",
                "sizing",
                "prependHeadingContext",
                "topicThreshold",
            ],
        ),
        ("ExtractionConfig", vec!["useCache", "enableQualityProcessing"]),
    ]
    .iter()
    .cloned()
    .collect();

    // Derive the set of struct names for quick lookup during detection
    let struct_names: Vec<&str> = optional_fields.keys().cloned().collect();

    let lines: Vec<&str> = source.lines().collect();
    let mut result = String::with_capacity(source.len());
    let mut i = 0;
    let mut changes_made = false;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        // Check if this line starts a const constructor for a known type with optional fields.
        // Match patterns like `const EmbeddingConfig({` or `const EmbeddingConfig(`
        let struct_name_opt = struct_names
            .iter()
            .find(|&&name| {
                trimmed.starts_with(&format!("const {}({{", name)) || trimmed.starts_with(&format!("const {}(", name))
            })
            .cloned();

        if let Some(struct_name) = struct_name_opt {
            let fields_to_make_optional = optional_fields[struct_name].clone();
            eprintln!("[ALEF DEBUG] Found struct: {} with fields to make optional: {:?}", struct_name, fields_to_make_optional);

            // Emit the constructor opening line
            result.push_str(line);
            result.push('\n');
            i += 1;

            // Process parameter lines within the constructor
            while i < lines.len() {
                let param_line = lines[i];
                let param_trimmed = param_line.trim_start();

                // Check if this line closes the constructor
                if param_trimmed.starts_with("});") {
                    result.push_str(param_line);
                    result.push('\n');
                    i += 1;
                    break;
                }

                // Check if this is a parameter that should be made optional
                let mut modified = false;
                for &field_name in &fields_to_make_optional {
                    if param_trimmed.contains(&format!("required this.{}", field_name)) {
                        // Replace "required this.field" with "this.field"
                        let modified_line = param_line.replace(
                            &format!("required this.{}", field_name),
                            &format!("this.{}", field_name),
                        );
                        eprintln!("[ALEF DEBUG]   Modified {} field: '{}' -> '{}'", field_name, param_line.trim(), modified_line.trim());
                        result.push_str(&modified_line);
                        result.push('\n');
                        modified = true;
                        changes_made = true;
                        break;
                    }
                }

                if !modified {
                    // Not a field we're making optional, keep as-is
                    result.push_str(param_line);
                    result.push('\n');
                }
                i += 1;
            }
        } else {
            result.push_str(line);
            result.push('\n');
            i += 1;
        }
    }

    if changes_made {
        eprintln!("[ALEF DEBUG] make_struct_fields_with_defaults_optional: MADE CHANGES");
    } else {
        eprintln!("[ALEF DEBUG] make_struct_fields_with_defaults_optional: NO CHANGES");
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_struct_payload_uses_payload_derived_name() {
        let input = r#"sealed class FormatMetadata with _$FormatMetadata {
  const FormatMetadata._();

  const factory FormatMetadata.pdf({required PdfMetadata field0}) =
      FormatMetadata_Pdf;
  const factory FormatMetadata.docx({required DocxMetadata field0}) =
      FormatMetadata_Docx;
}
"#;
        let out = rewrite_frb_sealed_variants(input);
        assert!(
            out.contains("required PdfMetadata metadata"),
            "PdfMetadata payload should be named `metadata`, got:\n{out}"
        );
        assert!(
            out.contains("required DocxMetadata metadata"),
            "DocxMetadata payload should be named `metadata`, got:\n{out}"
        );
        assert!(!out.contains("field0"), "no `field0` should remain, got:\n{out}");
    }

    #[test]
    fn primitive_payload_uses_value_name() {
        let input = r#"  const factory OutputFormat.custom({required String field0}) =
      OutputFormat_Custom;
"#;
        let out = rewrite_frb_sealed_variants(input);
        assert!(
            out.contains("required String value"),
            "String payload should be named `value`, got:\n{out}"
        );
        assert!(!out.contains("field0"), "no `field0` should remain, got:\n{out}");
    }

    #[test]
    fn multi_field_tuple_uses_value0_value1() {
        let input = r#"  const factory Point.xy({required int field0, required int field1}) =
      Point_Xy;
"#;
        let out = rewrite_frb_sealed_variants(input);
        assert!(
            out.contains("required int value0"),
            "first tuple field should be `value0`, got:\n{out}"
        );
        assert!(
            out.contains("required int value1"),
            "second tuple field should be `value1`, got:\n{out}"
        );
        assert!(!out.contains("field0"), "no `field0` should remain, got:\n{out}");
        assert!(!out.contains("field1"), "no `field1` should remain, got:\n{out}");
    }

    #[test]
    fn named_struct_field_is_preserved() {
        // frb already emits the struct field name when the variant carries a
        // named struct payload (struct variant, not tuple variant). The
        // rewriter must not touch these.
        let input = r#"  const factory Shape.rect({required double width, required double height}) =
      Shape_Rect;
"#;
        let out = rewrite_frb_sealed_variants(input);
        assert!(
            out.contains("required double width"),
            "named field `width` must be preserved, got:\n{out}"
        );
        assert!(
            out.contains("required double height"),
            "named field `height` must be preserved, got:\n{out}"
        );
    }

    #[test]
    fn non_variant_lines_are_untouched() {
        // Regular function declarations, top-level classes, comments, etc.
        // must round-trip unchanged.
        let input = r#"// This file is automatically generated.
import 'package:freezed_annotation/freezed_annotation.dart';

Future<int> extractBytes({required List<int> content}) =>
    RustLib.instance.api.crateExtractBytes(content: content);

class Foo {
  final int field0;
  Foo({required this.field0});
}
"#;
        let out = rewrite_frb_sealed_variants(input);
        assert_eq!(out, input, "non-variant code must round-trip unchanged");
    }

    #[test]
    fn fallback_when_prefix_does_not_match_uses_value() {
        // Variant `Image` with payload `Bitmap` does not share a prefix; the
        // helper should fall back to `value` rather than producing something
        // nonsensical.
        let input = r#"  const factory Drawable.image({required Bitmap field0}) =
      Drawable_Image;
"#;
        let out = rewrite_frb_sealed_variants(input);
        assert!(
            out.contains("required Bitmap value"),
            "unrelated payload type should fall back to `value`, got:\n{out}"
        );
    }

    #[test]
    fn nullable_payload_strips_question_mark_for_inference() {
        let input = r#"  const factory Either.left({required LeftValue? field0}) =
      Either_Left;
"#;
        let out = rewrite_frb_sealed_variants(input);
        // `LeftValue?` starts with variant prefix `Left` so remainder is `Value`.
        assert!(
            out.contains("required LeftValue? value"),
            "nullable payload with prefix-matching type should produce `value`, got:\n{out}"
        );
    }

    #[test]
    fn realistic_sample_crate_format_metadata_block() {
        // Mirror the actual frb output shape from the sample_core fixture.
        let input = r#"sealed class FormatMetadata with _$FormatMetadata {
  const FormatMetadata._();

  const factory FormatMetadata.pdf({required PdfMetadata field0}) =
      FormatMetadata_Pdf;
  const factory FormatMetadata.docx({required DocxMetadata field0}) =
      FormatMetadata_Docx;
  const factory FormatMetadata.excel({required ExcelMetadata field0}) =
      FormatMetadata_Excel;
  const factory FormatMetadata.code({required String field0}) =
      FormatMetadata_Code;
}
"#;
        let out = rewrite_frb_sealed_variants(input);
        assert!(out.contains("required PdfMetadata metadata"));
        assert!(out.contains("required DocxMetadata metadata"));
        assert!(out.contains("required ExcelMetadata metadata"));
        assert!(out.contains("required String value"));
        assert!(
            !out.contains("field0"),
            "all `field0` occurrences must be rewritten, got:\n{out}"
        );
        // Structural lines preserved.
        assert!(out.contains("sealed class FormatMetadata"));
        assert!(out.contains("FormatMetadata_Pdf"));
    }

    #[test]
    fn idempotent_when_run_twice() {
        let input = r#"  const factory FormatMetadata.pdf({required PdfMetadata field0}) =
      FormatMetadata_Pdf;
"#;
        let once = rewrite_frb_sealed_variants(input);
        let twice = rewrite_frb_sealed_variants(&once);
        assert_eq!(once, twice, "rewriter must be idempotent");
    }

    #[test]
    fn multiple_distinct_sealed_class_variants_all_rewritten() {
        // Two distinct sealed classes in the same source file — the regex must
        // handle multiple non-overlapping matches without interference.
        let input = r#"sealed class FormatMetadata with _$FormatMetadata {
  const FormatMetadata._();

  const factory FormatMetadata.pdf({required PdfMetadata field0}) =
      FormatMetadata_Pdf;
  const factory FormatMetadata.docx({required DocxMetadata field0}) =
      FormatMetadata_Docx;
}

sealed class OutputFormat with _$OutputFormat {
  const OutputFormat._();

  const factory OutputFormat.custom({required String field0}) =
      OutputFormat_Custom;
  const factory OutputFormat.json({required JsonConfig field0}) =
      OutputFormat_Json;
}
"#;
        let out = rewrite_frb_sealed_variants(input);
        // FormatMetadata variants
        assert!(
            out.contains("required PdfMetadata metadata"),
            "PdfMetadata should become metadata, got:\n{out}"
        );
        assert!(
            out.contains("required DocxMetadata metadata"),
            "DocxMetadata should become metadata, got:\n{out}"
        );
        // OutputFormat variants
        assert!(
            out.contains("required String value"),
            "String should become value, got:\n{out}"
        );
        // `JsonConfig` shares the `Json` prefix with variant `Json`, so the remainder
        // `Config` is lower-cased to `config`.
        assert!(
            out.contains("required JsonConfig config"),
            "JsonConfig payload (Json prefix → Config remainder) should become `config`, got:\n{out}"
        );
        assert!(!out.contains("field0"), "no `field0` should remain, got:\n{out}");
    }

    /// A minimal `frb_generated.dart` carrying the FRB entrypoint + loader config,
    /// mirroring the real flutter_rust_bridge 2.x output shape.
    fn frb_generated_fixture() -> &'static str {
        r#"// @generated by `flutter_rust_bridge`@ 2.12.0.

import 'dart:async';
import 'dart:convert';
import 'frb_generated.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';

class RustLib extends BaseEntrypoint<RustLibApi, RustLibApiImpl, RustLibWire> {
  RustLib._();

  /// Initialize flutter_rust_bridge
  static Future<void> init({
    RustLibApi? api,
    BaseHandler? handler,
    ExternalLibrary? externalLibrary,
    bool forceSameCodegenVersion = true,
  }) async {
    await instance.initImpl(
      api: api,
      handler: handler,
      externalLibrary: externalLibrary,
      forceSameCodegenVersion: forceSameCodegenVersion,
    );
  }

  static const kDefaultExternalLibraryLoaderConfig =
      ExternalLibraryLoaderConfig(
        stem: 'sample_router_dart',
        ioDirectory: 'rust/target/release/',
        webPrefix: 'pkg/',
        wasmBindgenName: 'wasm_bindgen',
      );
}
"#
    }

    #[test]
    fn loader_rewrite_injecsample_package_relative_resolution() {
        let out = rewrite_frb_external_library_loader(
            frb_generated_fixture(),
            "sample_router",
            "sample_router",
            "sample_router_dart",
        );
        assert!(
            out.contains("externalLibrary ??= await _alefResolveExternalLibrary();"),
            "init must resolve the package-relative library, got:\n{out}"
        );
        assert!(
            out.contains("Isolate.resolvePackageUri(Uri.parse('package:sample_router/sample_router.dart'))"),
            "loader must resolve the package URI, got:\n{out}"
        );
        assert!(
            out.contains("src/sample_router_bridge_generated/"),
            "loader must target the bridge-generated dir, got:\n{out}"
        );
        assert!(
            out.contains("'libsample_router_dart.dylib'"),
            "missing macOS candidate, got:\n{out}"
        );
        assert!(
            out.contains("'libsample_router_dart.so'"),
            "missing linux candidate, got:\n{out}"
        );
        assert!(
            out.contains("'sample_router_dart.dll'"),
            "missing windows candidate, got:\n{out}"
        );
        assert!(out.contains("import 'dart:io';"), "must import dart:io, got:\n{out}");
        assert!(
            out.contains("import 'dart:isolate';"),
            "must import dart:isolate, got:\n{out}"
        );
    }

    #[test]
    fn loader_rewrite_is_idempotent() {
        let once = rewrite_frb_external_library_loader(
            frb_generated_fixture(),
            "sample_router",
            "sample_router",
            "sample_router_dart",
        );
        let twice = rewrite_frb_external_library_loader(&once, "sample_router", "sample_router", "sample_router_dart");
        assert_eq!(once, twice, "loader rewrite must be idempotent");
        assert_eq!(
            twice.matches("import 'dart:io';").count(),
            1,
            "imports must not duplicate"
        );
        assert_eq!(
            twice.matches("_alefResolveExternalLibrary() async").count(),
            1,
            "helper must not be injected twice"
        );
    }

    #[test]
    fn loader_rewrite_is_noop_without_init_prologue() {
        // lib.dart has no FRB entrypoint — must round-trip unchanged.
        let input = "// just some dart\nFuture<int> foo() async => 1;\n";
        assert_eq!(
            rewrite_frb_external_library_loader(input, "sample_router", "sample_router", "sample_router_dart"),
            input
        );
    }

    #[test]
    fn sealed_variant_rewrite_also_applies_loader_fix_via_stem() {
        // `rewrite_frb_sealed_variants` (the wired post-processor) must apply the
        // loader fix when the file carries the FRB loader config, deriving the
        // package/module from the embedded stem.
        let out = rewrite_frb_sealed_variants(frb_generated_fixture());
        assert!(
            out.contains("externalLibrary ??= await _alefResolveExternalLibrary();"),
            "sealed-variant pass must also inject the loader, got:\n{out}"
        );
        assert!(
            out.contains("Isolate.resolvePackageUri(Uri.parse('package:sample_router/sample_router.dart'))"),
            "package derived from stem must be `sample_router`, got:\n{out}"
        );
    }

    #[test]
    fn sealed_variant_rewrite_leaves_lib_dart_loader_untouched() {
        // lib.dart (no loader config) must not gain a loader injection.
        let input = r#"import 'frb_generated.dart';

Future<int> extractBytes({required List<int> content}) =>
    RustLib.instance.api.crateExtractBytes(content: content);
"#;
        let out = rewrite_frb_sealed_variants(input);
        assert!(
            !out.contains("_alefResolveExternalLibrary"),
            "lib.dart must not get a loader, got:\n{out}"
        );
    }

    #[test]
    fn loader_rewrite_includes_rid_aware_path() {
        let out = rewrite_frb_external_library_loader(
            frb_generated_fixture(),
            "sample_router",
            "sample_router",
            "sample_router_dart",
        );
        assert!(
            out.contains("src/native/"),
            "loader must check RID-aware path (src/native/<rid>/), got:\n{out}"
        );
        assert!(
            out.contains("computeRid()"),
            "loader must compute RID from platform and arch, got:\n{out}"
        );
        assert!(
            out.contains("Platform.operatingSystem"),
            "loader must detect operating system, got:\n{out}"
        );
        assert!(
            out.contains("'linux-x64'") || out.contains("linux-"),
            "loader must support linux RID variants, got:\n{out}"
        );
        assert!(
            out.contains("'macos-arm64'") || out.contains("macos-"),
            "loader must support macos RID variants, got:\n{out}"
        );
        assert!(
            out.contains("'windows-x64'") || out.contains("windows-"),
            "loader must support windows RID variants, got:\n{out}"
        );
        // RID-aware check should come before legacy fallback
        let rid_pos = out.find("src/native/").expect("RID path must exist");
        let legacy_pos = out
            .find("src/sample_router_bridge_generated/")
            .expect("legacy path must exist");
        assert!(
            rid_pos < legacy_pos,
            "RID-aware check must come before legacy fallback, got:\n{out}"
        );
    }

    #[test]
    fn filter_excluded_functions_removes_multiline_functions() {
        let input = r#"/// Pass `metadata` as `null` when the caller has no extraction metadata available;
/// the metadata bonus simply isn't applied in that case. Texts shorter than
/// `MIN_TEXT_LENGTH` short-circuit to `0.1` regardless of metadata.
Future<double> calculateQualityScore({
  required String text,
  Map<String, String>? metadata,
}) => RustLib.instance.api.crateCalculateQualityScore(
  text: text,
  metadata: metadata,
);

Future<ExtractionResult> extractBytes(
  {required Uint8List content, required String mimeType}) =>
    RustLib.instance.api.crateExtractBytes(content: content, mimeType: mimeType);
"#;
        let exclude_set = std::collections::HashSet::from(["calculate_quality_score"]);
        let out = filter_excluded_functions(input, &exclude_set);

        // The excluded function and its doc comments should be removed
        assert!(
            !out.contains("calculateQualityScore"),
            "excluded function calculateQualityScore must be removed, got:\n{out}"
        );
        assert!(
            !out.contains("MIN_TEXT_LENGTH"),
            "doc comments for excluded function must be removed, got:\n{out}"
        );

        // Other functions should remain
        assert!(
            out.contains("extractBytes"),
            "non-excluded function extractBytes must remain, got:\n{out}"
        );
    }

    #[test]
    fn make_embedding_config_fields_with_defaults_optional() {
        let input = r#"class EmbeddingConfig {
  /// The embedding model to use (defaults to "balanced" preset if not specified)
  final EmbeddingModelType model;

  /// Whether to normalize embedding vectors (recommended for cosine similarity)
  final bool normalize;

  /// Batch size for embedding generation
  final PlatformInt64 batchSize;

  /// Show model download progress
  final bool showDownloadProgress;

  /// Custom cache directory for model files
  final String? cacheDir;

  /// Hardware acceleration for the embedding ONNX model.
  final AccelerationConfig? acceleration;

  /// Maximum wall-clock duration (in seconds) for a single `embed()` call.
  final PlatformInt64? maxEmbedDurationSecs;

  const EmbeddingConfig({
    required this.model,
    required this.normalize,
    required this.batchSize,
    required this.showDownloadProgress,
    this.cacheDir,
    this.acceleration,
    this.maxEmbedDurationSecs,
  });
}
"#;
        let out = make_struct_fields_with_defaults_optional(input);

        // Fields with defaults should be made optional
        assert!(
            out.contains("this.model,") && !out.contains("required this.model"),
            "model field should be made optional, got:\n{out}"
        );
        assert!(
            out.contains("this.normalize,") && !out.contains("required this.normalize"),
            "normalize field should be made optional, got:\n{out}"
        );
        assert!(
            out.contains("this.batchSize,") && !out.contains("required this.batchSize"),
            "batchSize field should be made optional, got:\n{out}"
        );
        assert!(
            out.contains("this.showDownloadProgress,") && !out.contains("required this.showDownloadProgress"),
            "showDownloadProgress field should be made optional, got:\n{out}"
        );

        // Already-optional fields should not be changed
        assert!(
            out.contains("this.cacheDir,"),
            "cacheDir field should remain, got:\n{out}"
        );
    }

    #[test]
    fn make_embedding_config_fields_optional_with_frb_formatting() {
        // FRB generates the opening brace on the next line
        let input = r#"class EmbeddingConfig {
  final EmbeddingModelType model;
  final bool normalize;
  final PlatformInt64 batchSize;
  final bool showDownloadProgress;

  const EmbeddingConfig({
    required this.model,
    required this.normalize,
    required this.batchSize,
    required this.showDownloadProgress,
  });
}
"#;
        let out = make_struct_fields_with_defaults_optional(input);

        // Fields should be made optional
        assert!(
            out.contains("this.model,") && !out.contains("required this.model"),
            "model field should be made optional, got:\n{out}"
        );
        assert!(
            out.contains("this.normalize,") && !out.contains("required this.normalize"),
            "normalize field should be made optional, got:\n{out}"
        );
        assert!(
            out.contains("this.batchSize,") && !out.contains("required this.batchSize"),
            "batchSize field should be made optional, got:\n{out}"
        );
        assert!(
            out.contains("this.showDownloadProgress,") && !out.contains("required this.showDownloadProgress"),
            "showDownloadProgress field should be made optional, got:\n{out}"
        );
    }

    #[test]
    fn prologue_replacement_helper_closes_before_init_opens() {
        let replacement = frb_init_prologue_replacement("test_pkg", "test_mod", "test_stem");

        // The format string uses {{ and }} to escape braces (literal { and } in output).
        // Count escaped braces: each {{ becomes { and each }} becomes }.
        let escaped_open = replacement.matches("{{").count();
        let escaped_close = replacement.matches("}}").count();
        assert_eq!(
            escaped_open, escaped_close,
            "escaped brace mismatch (must be in pairs for format escaping): {} {{ vs {} }}",
            escaped_open, escaped_close
        );

        // Check structure: the helper method should close with }} before init() opens.
        // Look for the sequence: `} catch (_) { ... } return null; } ... init({`
        assert!(
            replacement.contains("static Future<ExternalLibrary?> _alefResolveExternalLibrary()"),
            "helper method signature must exist"
        );
        assert!(
            replacement.contains("static Future<void> init({"),
            "init method signature must exist"
        );

        // Verify the closing }} for the helper precedes the init opening
        let helper_ret_null = replacement.find("return null;").expect("helper must have return null");
        let init_sig = replacement
            .find("static Future<void> init({")
            .expect("init sig must exist");
        assert!(helper_ret_null < init_sig, "helper return must precede init signature");
    }
}
