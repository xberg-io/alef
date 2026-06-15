//! Shared Dart native library staging logic for both build-time and publish-time.
//!
//! Both `cargo build` (post-build step) and `cargo publish` (packaging step) need
//! to stage prebuilt native libraries into the Dart package's `lib/src/native/<rid>/`
//! directory so that flutter_rust_bridge can find them at runtime.

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::warn;

/// Recursively copy a directory and all its contents.
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst).context(format!("creating directory: {}", dst.display()))?;
    for entry in fs::read_dir(src).context(format!("reading directory: {}", src.display()))? {
        let entry = entry.context(format!("reading entry in {}", src.display()))?;
        let path = entry.path();
        let file_name = entry.file_name();
        let dst_path = dst.join(&file_name);
        if path.is_dir() {
            copy_dir_recursive(&path, &dst_path)?;
        } else {
            fs::copy(&path, &dst_path)
                .with_context(|| format!("copying file {} to {}", path.display(), dst_path.display()))?;
        }
    }
    Ok(())
}

/// Platform-specific native library filename patterns.
/// Maps from runtime identifier (RID) to expected library filenames.
#[derive(Debug, Clone)]
struct NativeLibPattern {
    rid: &'static str,
    rust_target: &'static str,
    formats: &'static [NativeLibFormat],
}

#[derive(Debug, Clone, Copy)]
enum NativeLibFormat {
    MacosDylib,
    MacosFramework,
    UnixSharedObject,
    WindowsDll,
}

impl NativeLibFormat {
    fn filename(self, stem: &str) -> String {
        match self {
            Self::MacosDylib => format!("lib{stem}.dylib"),
            Self::MacosFramework => format!("{stem}.framework/{stem}"),
            Self::UnixSharedObject => format!("lib{stem}.so"),
            Self::WindowsDll => format!("{stem}.dll"),
        }
    }
}

const MACOS_NATIVE_LIB_FORMATS: &[NativeLibFormat] = &[NativeLibFormat::MacosDylib, NativeLibFormat::MacosFramework];
const LINUX_NATIVE_LIB_FORMATS: &[NativeLibFormat] = &[NativeLibFormat::UnixSharedObject];
const WINDOWS_NATIVE_LIB_FORMATS: &[NativeLibFormat] = &[NativeLibFormat::WindowsDll];

const NATIVE_LIB_PATTERNS: &[NativeLibPattern] = &[
    NativeLibPattern {
        rid: "macos-x64",
        rust_target: "x86_64-apple-darwin",
        formats: MACOS_NATIVE_LIB_FORMATS,
    },
    NativeLibPattern {
        rid: "macos-arm64",
        rust_target: "aarch64-apple-darwin",
        formats: MACOS_NATIVE_LIB_FORMATS,
    },
    NativeLibPattern {
        rid: "linux-x64",
        rust_target: "x86_64-unknown-linux-gnu",
        formats: LINUX_NATIVE_LIB_FORMATS,
    },
    NativeLibPattern {
        rid: "linux-arm64",
        rust_target: "aarch64-unknown-linux-gnu",
        formats: LINUX_NATIVE_LIB_FORMATS,
    },
    NativeLibPattern {
        rid: "windows-x64",
        rust_target: "x86_64-pc-windows-msvc",
        formats: WINDOWS_NATIVE_LIB_FORMATS,
    },
    NativeLibPattern {
        rid: "windows-arm64",
        rust_target: "aarch64-pc-windows-msvc",
        formats: WINDOWS_NATIVE_LIB_FORMATS,
    },
];

/// Find prebuilt native libraries in the cargo target directory.
/// Searches `{workspace}/target/{rust_target}/release/` for expected library filenames.
fn find_native_libraries(
    workspace_root: &Path,
    rust_target: &str,
    filenames: &[String],
) -> Result<Vec<(PathBuf, PathBuf)>> {
    let target_dir = workspace_root.join("target").join(rust_target).join("release");
    let mut found = Vec::new();

    if !target_dir.exists() {
        return Ok(found); // Target not yet built; OK to skip
    }

    for filename in filenames {
        let relative_path = PathBuf::from(filename);
        let lib_path = target_dir.join(&relative_path);
        if lib_path.exists() {
            found.push((lib_path, relative_path));
        }
    }

    Ok(found)
}

/// Stage prebuilt native libraries into a Dart package's lib/src/native/ directory.
///
/// Creates `{package_root}/lib/src/native/{rid}/` and copies native libraries there.
/// If no native libraries are found, this is a no-op (development builds may lack them).
///
/// Arguments:
/// - `workspace_root`: Root of the workspace (where `target/` and Cargo.toml are)
/// - `package_root`: Root of the Dart package (where `pubspec.yaml` is; often `{workspace_root}/packages/dart`)
/// - `stem`: The library name stem (e.g., `sample_lib_dart` for a `libsample_lib_dart.dylib`)
pub fn stage_dart_native_libraries(workspace_root: &Path, package_root: &Path, stem: &str) -> Result<()> {
    let native_base = package_root.join("lib/src/native");
    let mut staged_any = false;

    for pattern in NATIVE_LIB_PATTERNS {
        let filenames = pattern
            .formats
            .iter()
            .map(|format| format.filename(stem))
            .collect::<Vec<_>>();
        let libs = find_native_libraries(workspace_root, pattern.rust_target, &filenames)?;
        if libs.is_empty() {
            continue;
        }

        // Create RID-specific directory
        let rid_dir = native_base.join(pattern.rid);
        fs::create_dir_all(&rid_dir).context(format!("creating native library directory: {}", rid_dir.display()))?;

        // Copy each library
        for (lib_path, relative_path) in libs {
            let dest = rid_dir.join(relative_path);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("creating native library parent directory: {}", parent.display()))?;
            }
            // Handle directories (e.g., .framework) separately from files
            if lib_path.is_dir() {
                copy_dir_recursive(&lib_path, &dest).with_context(|| {
                    format!(
                        "copying native library directory {} to {}",
                        lib_path.display(),
                        dest.display()
                    )
                })?;
            } else {
                fs::copy(&lib_path, &dest)
                    .with_context(|| format!("copying native library {} to {}", lib_path.display(), dest.display()))?;
            }
            staged_any = true;
        }
    }

    // If no native libraries were found across all platforms, log a warning but don't fail.
    // This is normal during development or when the Rust crate hasn't been built yet.
    if !staged_any {
        warn!(
            "no prebuilt native libraries found for Dart binding '{}'; packages will require local build",
            stem
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_native_libraries_uses_package_stem() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let target_dir = tmp.path().join("target/aarch64-apple-darwin/release");
        fs::create_dir_all(&target_dir).unwrap();
        fs::write(target_dir.join("libmy_lib_dart.dylib"), "native").unwrap();

        let package_root = tmp.path().join("packages/dart");
        fs::create_dir_all(&package_root).unwrap();

        stage_dart_native_libraries(tmp.path(), &package_root, "my_lib_dart").unwrap();

        assert!(
            package_root
                .join("lib/src/native/macos-arm64/libmy_lib_dart.dylib")
                .exists()
        );
    }

    #[test]
    fn stage_native_libraries_preserves_framework_layout() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let target_dir = tmp
            .path()
            .join("target/aarch64-apple-darwin/release/my_lib_dart.framework");
        fs::create_dir_all(&target_dir).unwrap();
        fs::write(target_dir.join("my_lib_dart"), "native").unwrap();

        let package_root = tmp.path().join("packages/dart");
        fs::create_dir_all(&package_root).unwrap();

        stage_dart_native_libraries(tmp.path(), &package_root, "my_lib_dart").unwrap();

        assert!(
            package_root
                .join("lib/src/native/macos-arm64/my_lib_dart.framework/my_lib_dart")
                .exists()
        );
    }
}
