use regex::Regex;
use std::sync::OnceLock;

use super::imports_helpers::ensure_loader_imports;

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
        source.to_string()
    } else {
        let Some(prologue) = frb_init_prologue(source) else {
            return source.to_string();
        };
        let replacement = frb_init_prologue_replacement(package_name, module_name, stem);
        source.replacen(&prologue, &replacement, 1)
    };

    ensure_loader_imports(&with_loader, package_name)
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
pub(super) fn frb_init_prologue_replacement(package_name: &str, module_name: &str, stem: &str) -> String {
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
  /// 4. Versioned user cache populated by `dart run {package}:download_libs`
  ///    (`<cache>/{package}/<version>/<rid>/`), shared with the download script
  ///    via `nativeCachedLibPath()` in `native_loader.dart`.
  /// 5. Throws a StateError naming the expected release asset URL, the
  ///    download command, and the env-var override (never a silent null miss).
  static Future<ExternalLibrary?> {marker}() async {{
    try {{
      const candidates = <String>[
        // macOS: framework bundle (preferred modern packaging)
        '{stem}.framework',
        // macOS: bare dylib fallback
        'lib{stem}.dylib',
        // Linux
        'lib{stem}.so',
        // Windows
        '{stem}.dll',
      ];

      // Helper to open a native library by absolute path.
      // Normalizes path to absolute to avoid hardened-runtime "relative path rejected" errors.
      ExternalLibrary? tryOpenAbsolute(String libPath) {{
        try {{
          final absPath = File(libPath).absolute.path;
          return ExternalLibrary.open(absPath);
        }} catch (_) {{
          return null;
        }}
      }}

      bool candidateExists(String libPath) {{
        return File(libPath).existsSync() || Directory(libPath).existsSync();
      }}

      // Check FRB_DART_LOAD_EXTERNAL_LIBRARY_NATIVE_LIB_DIR env var first.
      // This allows test harnesses to override library location for development.
      final envDir = Platform.environment['FRB_DART_LOAD_EXTERNAL_LIBRARY_NATIVE_LIB_DIR'];
      if (envDir != null && envDir.isNotEmpty) {{
        final absEnvDir = Directory(envDir).absolute.path;
        final libDir = Directory(absEnvDir);
        if (libDir.existsSync()) {{
          for (final candidate in candidates) {{
            final libPath = '$absEnvDir/$candidate';
            if (candidateExists(libPath)) {{
              final result = tryOpenAbsolute(libPath);
              if (result != null) return result;
            }}
          }}
        }}
      }}

      // Compute RID (runtime identifier) from platform and architecture using Abi.current().
      // This is more reliable than parsing Platform.version.
      String? computeRid() {{
        final abi = Abi.current();
        final os = Platform.operatingSystem;

        // Map from (os, Abi) to RID string.
        String? ridFromAbi() {{
          if (os == 'linux') {{
            if (abi == Abi.linuxX64) return 'linux-x64';
            if (abi == Abi.linuxArm64) return 'linux-arm64';
          }} else if (os == 'macos') {{
            if (abi == Abi.macosX64) return 'macos-x64';
            if (abi == Abi.macosArm64) return 'macos-arm64';
          }} else if (os == 'windows') {{
            if (abi == Abi.windowsX64) return 'windows-x64';
            if (abi == Abi.windowsArm64) return 'windows-arm64';
          }}
          return null;
        }}

        return ridFromAbi();
      }}

      final rid = computeRid();
      if (rid != null) {{
        final packageRoot =
            await Isolate.resolvePackageUri(Uri.parse('package:{package}/{package}.dart'));
        if (packageRoot != null) {{
          final ridDir = packageRoot.resolve('src/native/$rid/');
          for (final candidate in candidates) {{
            final libPath = ridDir.resolve(candidate).toFilePath();
            if (candidateExists(libPath)) {{
              final result = tryOpenAbsolute(libPath);
              if (result != null) return result;
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
          if (candidateExists(libPath)) {{
            final result = tryOpenAbsolute(libPath);
            if (result != null) return result;
          }}
        }}
      }}

      // As a last resort, resolve the running test/script's package root via
      // `Platform.script` and search standard RID-relative locations there.
      // Critical on macOS: `Directory.current` under hardened-runtime `dart` is
      // the dart binary's own bin dir (relative-path dlopen rejected), whereas
      // `Platform.script` resolves to the running .dart file's absolute URI,
      // from which we can walk up to find the package root (the dir containing
      // `pubspec.yaml`) and look for the bundled native library at standard
      // paths. This handles the case where `Isolate.resolvePackageUri`
      // resolution did not yield the actual staging location (e.g., a path
      // dependency in local development, or a test_app whose host package
      // contains the native lib directly rather than via the bridged package).
      try {{
        final scriptPath = Platform.script.toFilePath();
        var dir = File(scriptPath).absolute.parent;
        while (dir.parent.path != dir.path
            && !File('${{dir.path}}/pubspec.yaml').existsSync()) {{
          dir = dir.parent;
        }}
        if (File('${{dir.path}}/pubspec.yaml').existsSync()) {{
          final rid = computeRid();
          final absRootPath = dir.absolute.path;
          final searchRoots = <String>[
            if (rid != null) '$absRootPath/lib/src/native/$rid',
            '$absRootPath/lib',
            absRootPath,
          ];
          for (final root in searchRoots) {{
            final absRoot = Directory(root).absolute.path;
            for (final candidate in candidates) {{
              final libPath = '$absRoot/$candidate';
              if (candidateExists(libPath)) {{
                final result = tryOpenAbsolute(libPath);
                if (result != null) return result;
              }}
            }}
          }}
        }}
      }} catch (_) {{
        // fall through to default loader
      }}

      // Versioned user cache populated by `dart run {package}:download_libs`.
      // Shares its cache-path logic with the download script via
      // `nativeCachedLibPath()` so the two can never disagree on the location.
      final cachedLibPath = nativeCachedLibPath();
      if (cachedLibPath != null && candidateExists(cachedLibPath)) {{
        final result = tryOpenAbsolute(cachedLibPath);
        if (result != null) return result;
      }}
    }} catch (_) {{
      // Fall through to the descriptive miss below on any resolution failure.
    }}

    // Nothing bundled and nothing staged in the cache: fail loudly rather than
    // let flutter_rust_bridge attempt a doomed relative-path dlopen. Name the
    // exact release asset, the fetch command, and the env-var override so the
    // consumer can recover.
    final rid = nativeComputeRid() ?? Platform.operatingSystem;
    throw StateError(
      'Native library for {package} ($rid) was not found. '
      'Expected it in the versioned cache (${{nativeCacheDir() ?? '<unresolved cache dir>'}}) '
      'or bundled with the package. Download it with '
      '`dart run {package}:download_libs`, which fetches '
      '${{nativeAssetUrlBase()}}.tar.gz and verifies its SHA-256, '
      'or point \$nativeLibDirEnv at a directory containing the native library.',
    );
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
pub(super) fn apply_loader_fix_from_stem(source: &str) -> String {
    let Some(stem) = extract_loader_stem(source) else {
        return source.to_string();
    };
    let crate_base = stem.strip_suffix("_dart").unwrap_or(&stem);
    let package_name = crate_base;
    let module_name = crate_base;
    rewrite_frb_external_library_loader(source, package_name, module_name, &stem)
}
