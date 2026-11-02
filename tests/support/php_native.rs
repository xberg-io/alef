//! Builds an ext-php-rs extension from generated binding source, for the native PHP fixtures.

use std::path::Path;

#[path = "mise_task.rs"]
mod mise_task;

/// Writes `source` as the crate root of a cdylib depending on ext-php-rs and builds it offline,
/// returning the built library. `ALEF_PHP_NATIVE_TARGET_DIR` shares one target directory.
pub fn build_php_extension(fixture: &Path, source: &str) -> std::path::PathBuf {
    std::fs::create_dir(fixture.join("src")).unwrap();
    std::fs::create_dir(fixture.join(".cargo")).unwrap();
    std::fs::write(
        fixture.join(".cargo/config.toml"),
        "[target.'cfg(target_os = \"macos\")']\nrustflags = [\"-C\", \"link-arg=-Wl,-undefined,dynamic_lookup\"]\n",
    )
    .unwrap();
    std::fs::write(fixture.join("src/lib.rs"), source).unwrap();
    std::fs::write(
        fixture.join("Cargo.toml"),
        "[package]\nname = \"alef-php-shared-probe\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[lib]\n\
         crate-type = [\"cdylib\"]\n[dependencies]\next-php-rs = \"=0.16.3\"\n",
    )
    .unwrap();
    std::fs::write(
        fixture.join("mise.toml"),
        "[tasks.build]\nrun = \"cargo build --offline\"\n",
    )
    .unwrap();
    let target = std::env::var_os("ALEF_PHP_NATIVE_TARGET_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| fixture.join("target"));
    let build = mise_task::mise_run(fixture, "build")
        .env("CARGO_TARGET_DIR", &target)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let library = target.join("debug").join(format!(
        "{}alef_php_shared_probe{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ));
    assert!(library.is_file(), "{}", library.display());
    library
}
