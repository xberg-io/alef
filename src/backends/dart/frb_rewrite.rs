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
    let with_loader = if source.contains(ALEF_LOADER_MARKER) {
        // Loader already injected on a prior run; keep the source verbatim but
        // still run `ensure_loader_imports` below so subsequent additions to
        // the required-imports set (e.g. the unprefixed `dart:core` rescue)
        // land in already-patched files without a full FRB regen.
        source.to_string()
    } else {
        let Some(prologue) = frb_init_prologue(source) else {
            return source.to_string();
        };
        let replacement = frb_init_prologue_replacement(package_name, module_name, stem);
        source.replacen(&prologue, &replacement, 1)
    };

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
        // macOS: framework bundle (preferred modern packaging)
        '{stem}.framework/{stem}',
        // macOS: bare dylib fallback
        'lib{stem}.dylib',
        // Linux
        'lib{stem}.so',
        // Windows
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
    // The aliased `import 'dart:core' as _DartCore;` SUPPRESSES the implicit
    // unprefixed `dart:core` import per the Dart spec, so without an explicit
    // unprefixed import every bare reference to `String`, `int`, `bool`,
    // `List`, `double`, … in the FRB-generated file would fail to resolve with
    // `Error: Type 'X' not found.`. We keep both: the unprefixed import
    // re-exposes the common types, and the aliased import lets us qualify just
    // `Uri` to avoid the FRB-generated `Uri` class collision.
    let needed = [
        ("import 'dart:core';", "import 'dart:core';\n"),
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
    // Use a sentinel-protected swap so a second pass over an already-rewritten file
    // doesn't double-qualify (`_DartCore.Uri.parse` → `_DartCore._DartCore.Uri.parse`).
    // Note: .resolve() is called on Uri instances, so it doesn't need qualification.
    const SENTINEL: &str = "\u{FEFF}__ALEF_URI_PARSE__\u{FEFF}";
    result = result.replace("_DartCore.Uri.parse(", SENTINEL);
    result = result.replace("Uri.parse(", "_DartCore.Uri.parse(");
    result = result.replace(SENTINEL, "_DartCore.Uri.parse(");

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

/// Fix FRB-generated Dart code that incorrectly calls `executeSync`/`executeNormal`
/// on callback function parameters.
///
/// When FRB generates service methods that take a callback function parameter
/// (e.g. `handler: FutureOr<String> Function(String)`), it emits code that calls
/// `handler.executeSync(...)` or `handler.executeNormal(...)`, but these methods
/// don't exist on function types. This rewrite strips the erroneous method calls,
/// calling the handler directly as a function.
///
/// FRB 2.x service-API callback parameters are plain `FutureOr<R> Function(T)` types,
/// not executor wrapper objects. The handler must be invoked directly: `await handler(arg)`.
/// This rewrite removes the erroneous `.executeSync()` / `.executeNormal()` method calls
/// that FRB incorrectly emits.
///
/// Additionally, any function/closure that contains `await handler(...)` calls must itself
/// be marked as `async`. This rewrite ensures all containing closures and methods are
/// properly declared as async.
///
/// Example transformation:
/// ```dart
/// // Before (FRB-generated, broken):
/// return handler.executeSync(
///   SyncTask(...),
/// );
///
/// // After (fixed):
/// return await handler(
///   SyncTask(...).request,
/// );
/// ```
///
/// Additionally, fixes FRB 2.x bug where `class RustLibApiImpl implements RustLibApi async`
/// is generated with an invalid `async` keyword in the class declaration. FRB generates this
/// incorrectly when the base class or mixin has async methods. The `async` keyword is only
/// valid on function declarations, not class declarations.
pub fn fix_handler_executor_calls(source: &str) -> String {
    // Strip the erroneous `.executeSync()` and `.executeNormal()` method calls
    // on callback function parameters. Replace them with direct invocation.
    // IMPORTANT: Only rewrite handler.execute* calls where `handler` is a parameter,
    // not where it's a class field (inherited from super.handler).

    // Pattern 4: Fix FRB 2.x bug where class declarations have invalid `async` keyword.
    // `class RustLibApiImpl implements RustLibApi async {` → `class RustLibApiImpl implements RustLibApi {`
    // The `async` keyword is only valid on functions, not class declarations.
    let mut result = source.replace(" implements RustLibApi async {", " implements RustLibApi {");

    // Rewrite handler.execute* calls only in functions/methods where `handler` is a parameter.
    result = rewrite_handler_calls_in_parameterized_functions(&result);

    // Pattern 3: Ensure closures/functions containing `await handler` are marked as async.
    // Fix patterns like: `({...}) {` to `({...}) async {` when body contains `await handler`.
    // This handles synchronous closure signatures that were not originally async.
    result = ensure_handler_closures_are_async(&result);

    result
}

/// Rewrite handler.executeSync/executeNormal calls, but ONLY within function/method
/// scopes where `handler` appears as a parameter.
///
/// This prevents rewriting `handler.execute*` on the class field `super.handler`,
/// which is NOT directly callable and should keep its execute* method calls.
fn rewrite_handler_calls_in_parameterized_functions(source: &str) -> String {
    let lines: Vec<&str> = source.lines().collect();
    let mut result = String::new();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];

        // Check if this line starts a function/method definition
        // Look for patterns: `... functionName(...)` or `... functionName(` with multi-line signature
        let is_function_start = is_likely_function_start(line);

        if is_function_start {
            // Check if this function has `handler` as a parameter
            let is_handler_parameterized = detect_handler_parameter(&lines, i);

            // Collect lines until we reach the closing brace of the function body.
            // Function signatures can span multiple lines before the opening `{`,
            // so keep collecting until the body starts and then closes.
            let mut func_lines = vec![line];
            i += 1;

            let mut depth = count_brace_depth(line);
            let mut saw_body = depth > 0;

            while i < lines.len() && (!saw_body || depth > 0) {
                let curr_line = lines[i];
                func_lines.push(curr_line);
                let line_depth = count_brace_depth(curr_line);
                depth += line_depth;
                saw_body = saw_body || line_depth > 0;
                i += 1;
            }

            // Rewrite if this function has handler parameter
            let func_text = func_lines.join("\n");
            let rewritten = if is_handler_parameterized {
                // When `handler` is a parameter, it's serialized and sent to Rust, where Rust invokes it.
                // The Dart code should NOT invoke the handler directly. Instead, it should invoke
                // the task executor (executeSync/executeNormal) on the task itself.
                //
                // Rewrite: `handler.executeSync(Task(...))` → `Task(...).executeSync()`
                // Rewrite: `handler.executeNormal(Task(...))` → `await Task(...).executeNormal()`
                rewrite_handler_to_task_executor(&func_text)
            } else {
                func_text
            };

            result.push_str(&rewritten);
            result.push('\n');
        } else {
            // Not a function start; just pass through
            result.push_str(line);
            result.push('\n');
            i += 1;
        }
    }

    // Remove the extra trailing newline if the original didn't have it
    if !source.ends_with('\n') && result.ends_with('\n') {
        result.pop();
    }

    result
}

/// Quick heuristic to detect if a line likely starts a function definition.
/// Looks for patterns: `Type name(` or `async {...` or `@override`
fn is_likely_function_start(line: &str) -> bool {
    let trimmed = line.trim();

    // Skip comments and empty lines
    if trimmed.is_empty() || trimmed.starts_with("//") {
        return false;
    }

    // @override always precedes function definitions
    if trimmed.starts_with("@") {
        return false; // The actual function is on the next line
    }

    // Function signatures typically have an opening paren
    if !line.contains('(') {
        return false;
    }

    // Exclude closing braces, field assignments, etc.
    if trimmed.starts_with("}") || trimmed.starts_with("]") || trimmed.starts_with(")") {
        return false;
    }

    // Check if line contains `{` or ends with the start of signature (maybe multi-line)
    // This is a heuristic and might match non-function-starting lines, which is OK
    // because we'll later filter by whether handler is a parameter
    true
}

/// Count the net braces in a line (positive = more opens than closes)
fn count_brace_depth(line: &str) -> i32 {
    let opens = line.chars().filter(|c| *c == '{').count() as i32;
    let closes = line.chars().filter(|c| *c == '}').count() as i32;
    opens - closes
}

/// Check if the function/method at line `idx` has `handler` as a parameter.
/// Looks for a function signature that includes `handler` in its parameter list.
fn detect_handler_parameter(lines: &[&str], idx: usize) -> bool {
    if idx >= lines.len() {
        return false;
    }

    let line = lines[idx];

    // Quick check: does this line contain both `(` and potentially the start of a parameter list?
    if !line.contains('(') {
        // This might be a multi-line signature; check the next few lines
        for l in lines.iter().take(std::cmp::min(idx + 20, lines.len())).skip(idx) {
            if l.contains("handler") && l.contains("Function") {
                // Likely contains `handler: ... Function(...)` parameter
                return true;
            }
            if l.contains(')') && l.contains('{') {
                // Reached the end of the signature; stop searching
                break;
            }
        }
    } else {
        // Single-line or start of multi-line signature on this line
        // Collect lines until we close the parameter list
        let mut sig = line.to_string();
        let mut paren_depth = line.chars().filter(|c| *c == '(').count() - line.chars().filter(|c| *c == ')').count();

        let mut j = idx + 1;
        while j < lines.len() && paren_depth > 0 {
            let l = lines[j];
            sig.push(' ');
            sig.push_str(l);
            paren_depth += l.chars().filter(|c| *c == '(').count();
            paren_depth -= l.chars().filter(|c| *c == ')').count();
            j += 1;
        }

        // Check if the signature contains `handler` as a parameter
        if sig.contains("handler") && sig.contains("Function") {
            return true;
        }
    }

    false
}

/// Rewrite handler.executeSync/executeNormal to move the method call to the task.
/// When handler is a parameter, FRB generates:
///   `return handler.executeSync(SyncTask(...));`
/// But the handler parameter can't be invoked directly (it's serialized and passed to Rust).
/// Instead, invoke the task executor:
///   `return SyncTask(...).executeSync();`
fn rewrite_handler_to_task_executor(source: &str) -> String {
    // Fix the pattern where FRB generates a stray closing paren before .executeSync()/.executeNormal()
    //
    // Pattern (raw from FRB with (?s) for dot matching newlines):
    //   ),\n  <-- Task constructor closing paren + comma
    //   ).executeSync();  <-- orphaned closing paren before the method call
    //
    // The `)` before `.executeSync()` is orphaned and should be removed.
    // Fix: Strip the orphaned `)` on the line before `.executeSync()` / `.executeNormal()`

    let mut result = rewrite_handler_executor_wrappers(source);

    // Match `),` followed by any whitespace (including newlines), then orphaned `)` before `.executeSync()` or `.executeNormal()`
    // Pattern: `),` + newline + indent + `)` + `.execute(Sync|Normal)()`
    let orphaned_paren_sync =
        Regex::new(r"(?s)\),\s*\)\.executeSync\(\)").expect("orphaned paren sync pattern must compile");
    result = orphaned_paren_sync.replace_all(&result, ").executeSync()").into_owned();

    let orphaned_paren_async =
        Regex::new(r"(?s)\),\s*\)\.executeNormal\(\)").expect("orphaned paren async pattern must compile");
    result = orphaned_paren_async
        .replace_all(&result, ").executeNormal()")
        .into_owned();

    result
}

fn rewrite_handler_executor_wrappers(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut cursor = 0;

    while let Some((relative_start, method)) = find_next_handler_executor(&source[cursor..]) {
        let start = cursor + relative_start;
        let open_paren = start + format!("handler.{method}").len();
        let Some(close_paren) = find_matching_paren(source, open_paren) else {
            break;
        };

        out.push_str(&source[cursor..start]);
        let task = source[open_paren + 1..close_paren].trim();
        let task = task.strip_suffix(',').map(str::trim_end).unwrap_or(task);
        out.push_str(task);
        out.push('.');
        out.push_str(method);
        out.push_str("()");
        cursor = close_paren + 1;
    }

    out.push_str(&source[cursor..]);
    out
}

fn find_next_handler_executor(source: &str) -> Option<(usize, &'static str)> {
    let sync = source.find("handler.executeSync(");
    let normal = source.find("handler.executeNormal(");

    match (sync, normal) {
        (Some(sync), Some(normal)) if sync <= normal => Some((sync, "executeSync")),
        (Some(_), Some(normal)) => Some((normal, "executeNormal")),
        (Some(sync), None) => Some((sync, "executeSync")),
        (None, Some(normal)) => Some((normal, "executeNormal")),
        (None, None) => None,
    }
}

fn find_matching_paren(source: &str, open_paren: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (offset, ch) in source[open_paren..].char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(open_paren + offset);
                }
            }
            _ => {}
        }
    }
    None
}

/// Ensure all closures and anonymous functions that contain `await handler` calls
/// are declared as `async`. This fixes the Dart compile error where `await` is used
/// in a non-async context.
fn ensure_handler_closures_are_async(source: &str) -> String {
    let lines: Vec<&str> = source.lines().collect();

    // First pass: identify which lines need `async` injected. For each line that
    // starts a function or closure, check if any of the next ~30 lines contain
    // `await handler`. If so, mark the closing brace line for mutation.
    let mut lines_to_fix: std::collections::HashSet<usize> = std::collections::HashSet::new();

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];

        // Skip comments, class/mixin declarations, and lines that already have async.
        // Class declarations (starting with `class `, `abstract class`, `mixin `, etc.) must
        // never receive an `async` keyword — `async` is only valid on function declarations.
        let trimmed_line = line.trim();
        if trimmed_line.starts_with("//")
            || line.contains("async")
            || trimmed_line.starts_with("class ")
            || trimmed_line.starts_with("abstract class ")
            || trimmed_line.starts_with("mixin ")
        {
            i += 1;
            continue;
        }

        // Check if any of the next ~30 lines contain `await handler`
        let contains_await_handler =
            (i..std::cmp::min(i + 30, lines.len())).any(|j| lines[j].contains("await handler("));

        if contains_await_handler {
            let parens_balanced =
                line.chars().filter(|c| *c == '(').count() == line.chars().filter(|c| *c == ')').count();

            // Case 1: Single-line signature with balanced parens and opening brace
            if parens_balanced && line.contains('{') {
                lines_to_fix.insert(i);
            }
            // Case 2: Multi-line signature (unbalanced parens) — find the closing brace line
            else if !parens_balanced {
                for (j, check_line) in lines
                    .iter()
                    .enumerate()
                    .take(std::cmp::min(i + 30, lines.len()))
                    .skip(i + 1)
                {
                    // Look for a line that has `)` (closing paren) and `{` (opening brace).
                    // This is typically the closing line of a multi-line function signature.
                    // Skip lines that already have `async` — adding it again would duplicate the keyword.
                    if check_line.contains(')') && check_line.contains('{') && !check_line.trim().starts_with("//") {
                        if !check_line.contains("async") {
                            lines_to_fix.insert(j);
                        }
                        break;
                    }
                }
            }
        }

        i += 1;
    }

    // Second pass: apply the fixes
    let mut result = String::new();
    for (i, line) in lines.iter().enumerate() {
        if lines_to_fix.contains(&i) {
            let fixed = if line.contains(") {") {
                line.replace(") {", ") async {")
            } else {
                // Insert `async` before `{`
                let trimmed = line.trim_end();
                if trimmed.ends_with("{") {
                    format!("{} async {{", trimmed.trim_end_matches('{').trim_end())
                } else {
                    line.to_string()
                }
            };
            result.push_str(&fixed);
        } else {
            result.push_str(line);
        }
        result.push('\n');
    }

    // Remove the extra trailing newline if the original didn't have it
    if !source.ends_with('\n') && result.ends_with('\n') {
        result.pop();
    }

    result
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
    // Disabled: renaming `field<N>` in the factory in lib.dart creates a
    // signature mismatch with the underlying `<Variant>_<Sub>` class in
    // lib.freezed.dart (which freezed's build_runner emits with the original
    // `field<N>` names, since we run build_runner before — not after — the
    // alef post-build rewrite). The mismatch surfaces at every test compile
    // as "constructor function type … isn't a subtype of …", blocking the
    // entire dart e2e suite. Until alef re-runs build_runner after the
    // rewrite (or the rewrite also patches lib.freezed.dart), keep the
    // factory params as the canonical `field<N>` so the two files stay in
    // sync. Callers retain the variant_pascal/type_name parameters for the
    // future fix.
    let _ = (type_name, variant_pascal, total_fields);
    format!("field{field_idx}")
}

/// Lower the first character of a PascalCase identifier; preserve the rest.
#[allow(dead_code)]
fn to_lower_camel(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Dart primitive / stdlib types that should map to a generic `value` name.
#[allow(dead_code)]
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

/// Legacy identity transform kept for old post-build processor references.
///
/// Dart default handling is emitted from IR metadata in the generated wrapper
/// layer. This post-FRB source rewriter has no API metadata, so it must not infer
/// defaults from product-specific class or field names.
pub fn make_struct_fields_with_defaults_optional(source: &str) -> String {
    source.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_struct_payload_preserves_field0() {
        // The factory `field0` is left in place — see `payload_param_name` for
        // the rationale. Renaming here without also patching lib.freezed.dart
        // creates a constructor-redirect type mismatch.
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
            out.contains("required PdfMetadata field0"),
            "PdfMetadata payload should keep `field0`, got:\n{out}"
        );
        assert!(
            out.contains("required DocxMetadata field0"),
            "DocxMetadata payload should keep `field0`, got:\n{out}"
        );
    }

    #[test]
    fn primitive_payload_preserves_field0() {
        let input = r#"  const factory OutputFormat.custom({required String field0}) =
      OutputFormat_Custom;
"#;
        let out = rewrite_frb_sealed_variants(input);
        assert!(
            out.contains("required String field0"),
            "String payload should keep `field0`, got:\n{out}"
        );
    }

    #[test]
    fn multi_field_tuple_preserves_field_indices() {
        let input = r#"  const factory Point.xy({required int field0, required int field1}) =
      Point_Xy;
"#;
        let out = rewrite_frb_sealed_variants(input);
        assert!(
            out.contains("required int field0"),
            "first tuple field should keep `field0`, got:\n{out}"
        );
        assert!(
            out.contains("required int field1"),
            "second tuple field should keep `field1`, got:\n{out}"
        );
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
    fn unrelated_payload_preserves_field0() {
        let input = r#"  const factory Drawable.image({required Bitmap field0}) =
      Drawable_Image;
"#;
        let out = rewrite_frb_sealed_variants(input);
        assert!(
            out.contains("required Bitmap field0"),
            "unrelated payload type should keep `field0`, got:\n{out}"
        );
    }

    #[test]
    fn nullable_payload_preserves_field0() {
        let input = r#"  const factory Either.left({required LeftValue? field0}) =
      Either_Left;
"#;
        let out = rewrite_frb_sealed_variants(input);
        assert!(
            out.contains("required LeftValue? field0"),
            "nullable payload should keep `field0`, got:\n{out}"
        );
    }

    #[test]
    fn realistic_sample_crate_format_metadata_block_preserves_field0() {
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
        assert!(out.contains("required PdfMetadata field0"));
        assert!(out.contains("required DocxMetadata field0"));
        assert!(out.contains("required ExcelMetadata field0"));
        assert!(out.contains("required String field0"));
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
    fn multiple_distinct_sealed_class_variants_preserve_field0() {
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
        assert!(out.contains("required PdfMetadata field0"));
        assert!(out.contains("required DocxMetadata field0"));
        assert!(out.contains("required String field0"));
        assert!(out.contains("required JsonConfig field0"));
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
            out.contains("Isolate.resolvePackageUri(_DartCore.Uri.parse('package:sample_router/sample_router.dart'))"),
            "loader must resolve the package URI, got:\n{out}"
        );
        assert!(
            out.contains("src/sample_router_bridge_generated/"),
            "loader must target the bridge-generated dir, got:\n{out}"
        );
        assert!(
            out.contains("'sample_router_dart.framework/sample_router_dart'"),
            "missing macOS framework candidate, got:\n{out}"
        );
        assert!(
            out.contains("'libsample_router_dart.dylib'"),
            "missing macOS dylib candidate, got:\n{out}"
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
            out.contains("Isolate.resolvePackageUri(_DartCore.Uri.parse('package:sample_router/sample_router.dart'))"),
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
    fn fix_handler_executor_calls_adds_async_to_closures() {
        // When `handler` is a callback function parameter, handler.executeNormal/executeSync
        // calls should be rewritten. The handler receives the task result, not the task itself,
        // so we invoke the task executor and let Rust handle invoking the callback.
        let input = r#"Future<String> processRequest(Request req, FutureOr<String> Function(String) handler) {
  return handler.executeNormal(
    SyncTask(request: req),
  );
}

int handleRoute(RouteData route, FutureOr<int> Function(String) handler) {
  return handler.executeSync(
    RouteTask(data: route),
  );
}
"#;
        let out = fix_handler_executor_calls(input);

        // The handler is serialized and passed to Rust, so we invoke the task executor, not the handler
        assert!(
            out.contains(".executeNormal();") || out.contains(".executeSync();"),
            "expected task executor method calls in output, got:\n{out}"
        );
        assert!(
            !out.contains("handler("),
            "handler callback should NOT be invoked directly (it's serialized), got:\n{out}"
        );
        assert!(
            !out.contains("handler.executeSync(") && !out.contains("handler.executeNormal("),
            "handler.executeSync/executeNormal method calls should be removed (moved to task), got:\n{out}"
        );

        // Verify no double-await
        assert!(
            !out.contains("await await"),
            "should not have duplicate awaits, got:\n{out}"
        );
    }

    #[test]
    fn fix_handler_executor_calls_preserves_super_handler_without_parameter() {
        // When a method does NOT have `handler` as a parameter, handler.execute* calls
        // refer to `super.handler` (the class field) and should NOT be rewritten.
        let input = r#"  @override
  App crateServiceApiAppNew() {
    return handler.executeSync(
      SyncTask(
        callFfi: () {
          final serializer = SseSerializer(generalizedFrbRustBinding);
          return pdeCallFfi(generalizedFrbRustBinding, serializer, funcId: 5)!;
        },
        codec: SseCodec(
          decodeSuccessData: sse_decode_app,
          decodeErrorData: null,
        ),
        constMeta: kConstMeta,
        argValues: [],
        apiImpl: this,
      ),
    );
  }
"#;
        let out = fix_handler_executor_calls(input);

        // The method has no `handler` parameter, so handler.executeSync should NOT be rewritten
        assert!(
            out.contains("handler.executeSync("),
            "handler.executeSync should be preserved when handler is not a parameter, got:\n{out}"
        );
        assert!(
            !out.contains("handler.executeNormal("),
            "no executeNormal should be present in input"
        );
    }

    #[test]
    fn fix_handler_executor_calls_rewrites_when_handler_is_parameter() {
        // When a method HAS `handler` as a parameter, handler.execute* calls should be rewritten
        let input = r#"  @override
  int crateServiceApiAppConnect({
    required App that,
    required String path,
    required FutureOr<String> Function(String) handler,
  }) {
    return handler.executeSync(
      SyncTask(
        callFfi: () {
          final serializer = SseSerializer(generalizedFrbRustBinding);
          sse_encode_App(that, serializer);
          sse_encode_String(path, serializer);
          return pdeCallFfi(generalizedFrbRustBinding, serializer, funcId: 1)!;
        },
        codec: SseCodec(
          decodeSuccessData: sse_decode_i_32,
          decodeErrorData: null,
        ),
        constMeta: kConstMeta,
        argValues: [that, path, handler],
        apiImpl: this,
      ),
    );
  }
"#;
        let out = fix_handler_executor_calls(input);

        // The method has `handler` as a parameter, so handler.executeSync should be rewritten
        // to call executeSync on the task instead (the handler is serialized, not invoked directly).
        assert!(
            !out.contains("handler.executeSync("),
            "handler.executeSync should be rewritten when handler is a parameter, got:\n{out}"
        );
        assert!(
            out.contains(".executeSync();"),
            "expected `.executeSync();` on the task in output (rewritten), got:\n{out}"
        );
        assert!(
            !out.contains("handler("),
            "handler callback should NOT be invoked directly (it's serialized), got:\n{out}"
        );
    }

    #[test]
    fn fix_handler_executor_calls_strips_trailing_comma_in_wrapper() {
        // The original FRB pattern is: handler.executeNormal(NormalTask(...),)
        // With parentheses on separate lines due to multiline formatting.
        // The rewrite must emit: NormalTask(...).executeNormal();
        let input = r#"  Future<DartHandlerHandler> crateServiceApiDartHandlerHandlerNew({
    required FutureOr<String> Function(String) handler,
  }) {
    return await handler.executeNormal(
      NormalTask(
        callFfi: (port_) {
          final serializer = SseSerializer(generalizedFrbRustBinding);
          sse_encode_DartFn_Inputs_String_Output_String_AnyhowException(
            handler,
            serializer,
          );
        },
        codec: SseCodec(
          decodeSuccessData: sse_decode_Auto_Owned_RustOpaque,
          decodeErrorData: null,
        ),
        constMeta: kConstMeta,
        argValues: [handler],
        apiImpl: this,
      ),
    );
  }
"#;
        let out = fix_handler_executor_calls(input);

        // Should be rewritten to have NormalTask at the start
        assert!(
            out.contains("return await") && out.contains("NormalTask("),
            "NormalTask should follow return await, got:\n{out}"
        );
        // Should NOT have the handler.executeNormal wrapper
        assert!(
            !out.contains("handler.executeNormal("),
            "handler.executeNormal should be removed, got:\n{out}"
        );
        // Should have .executeNormal(); on the task
        assert!(
            out.contains(".executeNormal();"),
            "should have .executeNormal(); on the task, got:\n{out}"
        );
        // Should not have double closing parens or other syntax errors
        assert!(
            !out.contains(")).executeNormal();"),
            "should not have double closing parens, got:\n{out}"
        );
    }

    #[test]
    fn fix_handler_executor_calls_removes_invalid_async_from_class_declaration() {
        let input = r#"class RustLibApiImpl extends RustLibApiImplPlatform implements RustLibApi async {
  RustLibApiImpl({
    required super.handler,
    required super.wire,
    required super.generalizedFrbRustBinding,
    required super.portManager,
  });

  @override
  int crateServiceApiAppConnect({
    required App that,
    required String path,
    required FutureOr<String> Function(String) handler,
  }) async {
    return await handler("test");
  }
}
"#;
        let out = fix_handler_executor_calls(input);

        // Verify the invalid `async` keyword is removed from class declaration
        assert!(
            !out.contains("implements RustLibApi async {"),
            "class declaration should not have `async` keyword, got:\n{out}"
        );
        assert!(
            out.contains("implements RustLibApi {"),
            "class declaration should have closing brace without `async`, got:\n{out}"
        );

        // Verify the method can still be async
        assert!(
            out.contains(") async {") || out.contains("async {"),
            "method signatures should still be able to use `async`, got:\n{out}"
        );
    }

    #[test]
    fn default_field_rewriter_is_identity_without_ir_metadata() {
        let input = r#"class GenericOptions {
  final String mode;
  final bool enabled;
  final PlatformInt64 retryCount;

  const GenericOptions({
    required this.mode,
    required this.enabled,
    required this.retryCount,
  });
}
"#;
        let out = make_struct_fields_with_defaults_optional(input);

        assert_eq!(out, input, "post-FRB rewriter must not infer defaults by class name");
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

    #[test]
    fn fix_handler_executor_strips_orphaned_paren_sync_task() {
        // FRB 2.x generates code with orphaned closing paren before .executeSync()
        // Pattern: `),\n).executeSync();`
        // Should become: `).executeSync();`
        let input = r#"  @override
  int crateServiceApiAppConnect({
    required App that,
    required String path,
    required FutureOr<String> Function(String) handler,
  }) {
    return
      SyncTask(
        callFfi: () {
          final serializer = SseSerializer(generalizedFrbRustBinding);
          sse_encode_App(that, serializer);
          sse_encode_String(path, serializer);
          return pdeCallFfi(generalizedFrbRustBinding, serializer, funcId: 1)!;
        },
        codec: SseCodec(
          decodeSuccessData: sse_decode_i_32,
          decodeErrorData: null,
        ),
        constMeta: kConstMeta,
        argValues: [that, path, handler],
        apiImpl: this,
      ),
    ).executeSync();
  }
"#;
        let out = fix_handler_executor_calls(input);

        // The orphaned `)` on the line before `.executeSync()` should be removed
        // Pattern `),\n)` should become just `)`
        assert!(
            out.contains(").executeSync();"),
            "should have ).executeSync(); without orphaned paren, got:\n{out}"
        );
        // Make sure we didn't accidentally create double parens or syntax errors
        let lint_check = out.matches(")).executeSync()").count();
        assert_eq!(
            lint_check, 0,
            "should not have )).executeSync() (double paren), got:\n{out}"
        );
    }

    #[test]
    fn fix_handler_executor_strips_orphaned_paren_async_task() {
        // Same fix for AsyncTask/.executeNormal() pattern
        let input = r#"  @override
  Future<String> crateServiceApiAppHandlerCall({
    required App that,
    required String path,
    required FutureOr<String> Function(String) handler,
  }) async {
    return await
      AsyncTask(
        callFfi: (port_) {
          final serializer = SseSerializer(generalizedFrbRustBinding);
          sse_encode_App(that, serializer);
          sse_encode_String(path, serializer);
          return pdeCallFfiAsync(generalizedFrbRustBinding, serializer, funcId: 42);
        },
        codec: SseCodec(
          decodeSuccessData: sse_decode_String,
          decodeErrorData: null,
        ),
        constMeta: kConstMeta,
        argValues: [that, path, handler],
        apiImpl: this,
      ),
    ).executeNormal();
  }
"#;
        let out = fix_handler_executor_calls(input);

        // The orphaned `)` should be removed
        assert!(
            out.contains(").executeNormal();"),
            "should have ).executeNormal(); without orphaned paren, got:\n{out}"
        );
        let lint_check = out.matches(")).executeNormal()").count();
        assert_eq!(
            lint_check, 0,
            "should not have )).executeNormal() (double paren), got:\n{out}"
        );
    }
}
