/// Ensure `dart:io`, `dart:isolate`, `dart:core`, and `dart:ffi` are imported (the loader
/// helper uses `File`, `Isolate`, `Uri`, and `Abi`), plus the shared
/// `package:{package_name}/src/native_loader.dart` (the loader override calls
/// its `nativeCachedLibPath()` / `nativeCacheDir()` / `nativeAssetUrlBase()`
/// helpers to resolve and describe the versioned cache). Inserts each missing
/// import after the first existing `import` line. Idempotent.
///
/// To avoid namespace conflict with the FRB-generated `Uri` class, imports
/// `dart:core.Uri` with an alias (`_DartCoreUri`), then replaces all
/// `Uri.parse()` and `Uri.resolve()` calls with the aliased name.
pub(super) fn ensure_loader_imports(source: &str, package_name: &str) -> String {
    let mut result = source.to_string();
    let helper_import = format!("import 'package:{package_name}/src/native_loader.dart';");
    let helper_import_line = format!("{helper_import}\n");
    let needed = [
        ("import 'dart:core';", "import 'dart:core';\n"),
        ("import 'dart:core' as _DartCore;", "import 'dart:core' as _DartCore;\n"),
        ("import 'dart:io';", "import 'dart:io';\n"),
        ("import 'dart:isolate';", "import 'dart:isolate';\n"),
        ("import 'dart:ffi';", "import 'dart:ffi';\n"),
        (helper_import.as_str(), helper_import_line.as_str()),
    ];

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

    const SENTINEL: &str = "\u{FEFF}__ALEF_URI_PARSE__\u{FEFF}";
    result = result.replace("_DartCore.Uri.parse(", SENTINEL);
    result = result.replace("Uri.parse(", "_DartCore.Uri.parse(");
    result = result.replace(SENTINEL, "_DartCore.Uri.parse(");

    result
}
