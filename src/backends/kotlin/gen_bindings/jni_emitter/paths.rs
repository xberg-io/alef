/// Resolve the Kotlin package for JNI-mode output.
///
/// Prefers `[crates.kotlin_android] package`, then `[crates.kotlin] package`,
/// then falls back to `config.kotlin_package()`.
pub(in crate::backends::kotlin) fn jni_kotlin_package(config: &ResolvedCrateConfig) -> String {
    config
        .kotlin_android
        .as_ref()
        .and_then(|a| a.package.clone())
        .or_else(|| config.kotlin.as_ref().and_then(|k| k.package.clone()))
        .unwrap_or_else(|| config.kotlin_package())
}

/// Resolve the output path for a JNI-mode Kotlin file.
///
/// Uses `[crates.output] kotlin_android` when available, otherwise falls
/// back to `[crates.output] kotlin`, and finally the conventional
/// `packages/kotlin/src/main/kotlin/<pkg>/` layout.
pub(in crate::backends::kotlin) fn jni_output_path(config: &ResolvedCrateConfig, filename: &str) -> PathBuf {
    if let Some(android_out) = config.output_for("kotlin_android") {
        return android_out.join(filename);
    }
    let kotlin_root = config
        .output_for("kotlin")
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "packages/kotlin".to_string());
    let package = jni_kotlin_package(config);
    let package_path = package.replace('.', "/");
    if config.explicit_output.kotlin.is_some() {
        PathBuf::from(&kotlin_root).join(filename)
    } else {
        PathBuf::from(&kotlin_root)
            .join("src/main/kotlin")
            .join(&package_path)
            .join(filename)
    }
}
