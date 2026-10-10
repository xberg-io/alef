use regex::Regex;
use std::sync::OnceLock;

use super::imports_helpers::ensure_loader_imports;
use crate::backends::dart::template_env;

/// Idempotency marker injected into `RustLib.init` by
/// [`rewrite_frb_external_library_loader`]. Presence of this token means the
/// loader override has already been applied.
const ALEF_LOADER_MARKER: &str = "_alefResolveExternalLibrary";

/// Sentinel present ONLY in the current loader template (the versioned-cache
/// resolution step calls `nativeCachedLibPath()`). A file that carries
/// [`ALEF_LOADER_MARKER`] but NOT this sentinel was injected by an older alef
/// and must be upgraded in place — otherwise the marker-based idempotency check
/// would freeze a stale loader forever, even across regenerations. (This is the
/// exact failure that shipped a broken cache-unaware loader in a released
/// binding: the download script populated the versioned cache, but the frozen
/// loader never looked there.)
const ALEF_LOADER_CURRENT_SENTINEL: &str = "nativeCachedLibPath()";

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
        if source.contains(ALEF_LOADER_CURRENT_SENTINEL) {
            // Already injected with the current template — genuine no-op.
            source.to_string()
        } else {
            // Stale loader injected by an older alef: replace the whole injected
            // region (helper method + any obsolete sibling helpers + the patched
            // `init` prologue up to the `externalLibrary ??=` line) with the
            // current template, preserving the original `init` body that follows.
            let replacement = frb_init_prologue_replacement(package_name, module_name, stem);
            match injected_loader_region_regex().find(source) {
                Some(m) => format!("{}{}{}", &source[..m.start()], replacement, &source[m.end()..]),
                None => source.to_string(),
            }
        }
    } else {
        let Some(prologue) = frb_init_prologue(source) else {
            return source.to_string();
        };
        let replacement = frb_init_prologue_replacement(package_name, module_name, stem);
        source.replacen(&prologue, &replacement, 1)
    };

    ensure_loader_imports(&with_loader, package_name)
}

/// Match the entire previously-injected loader region: from the helper's leading
/// doc comment (`/// Resolve the prebuilt native library`, stable across every
/// template version) through the injected `externalLibrary ??= await
/// _alefResolveExternalLibrary();` line inside `init`. Non-greedy so it stops at
/// the first such assignment. The original `init` body follows the match and is
/// left untouched.
fn injected_loader_region_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?s)  /// Resolve the prebuilt native library.*?    externalLibrary \?\?= await _alefResolveExternalLibrary\(\);\n",
        )
        .expect("injected loader region regex must compile")
    })
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
/// Renders `dart_init_prologue_replacement.jinja`, the single source of truth for the
/// injected prologue also used to build the `patch_published_loader` fallback embedded in
/// the generated dart-bridge crate's `build.rs` (see
/// `gen_rust_crate::cargo::dart_init_prologue_replacement`). Both call sites must stay on
/// this one template — a second, hand-written copy previously drifted and shipped a
/// version that couldn't reach `nativeDownloadAndCacheLibrary()`, breaking cold-cache
/// installs.
pub(super) fn frb_init_prologue_replacement(package_name: &str, module_name: &str, stem: &str) -> String {
    template_env::render(
        "dart_init_prologue_replacement.jinja",
        crate::alef_context! {
            package_name => package_name,
            module_name => module_name,
            stem => stem,
        },
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
/// bridge-module and library stem from the file's own
/// `kDefaultExternalLibraryLoaderConfig`.
///
/// alef's dart backend names the bridge cdylib `<crate>_dart` (the FRB `stem`)
/// and emits its bridge sources under `lib/src/<crate>_bridge_generated/`, so
/// the module name is recovered by stripping the trailing `_dart` from the stem.
///
/// `package_name` must be the resolved `[dart] pubspec_name` — it is only a
/// coincidence that it equals the crate base when `pubspec_name` is
/// unconfigured. Deriving it from the stem emitted
/// `package:<crate>/src/native_loader.dart` into every renamed package, an
/// import that resolves nowhere and takes the whole bridge down with it.
///
/// No-op when no loader config is present (returns `source` unchanged), so this
/// is safe to call on `lib.dart` as well as `frb_generated.dart`.
pub(super) fn apply_loader_fix_from_stem(source: &str, package_name: &str) -> String {
    let Some(stem) = extract_loader_stem(source) else {
        return source.to_string();
    };
    let module_name = stem.strip_suffix("_dart").unwrap_or(&stem).to_string();
    rewrite_frb_external_library_loader(source, package_name, &module_name, &stem)
}
