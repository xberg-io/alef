//! Elixir NIF precompiled binary packaging.
//!
//! Produces one tarball per (target × nif_version) combination in the format
//! expected by `RustlerPrecompiled`:
//!
//! `{lib}-v{version}-nif-{nif_version}-{target}.{ext}.tar.gz`
//!
//! where `{ext}` is `so` (Linux/macOS) or `dll` (Windows).
//!
//! Also provides `write_elixir_checksums()` to generate the
//! `checksum-Elixir.{App}.exs` file that RustlerPrecompiled validates.

use super::PackageArtifact;
use crate::platform::RustTarget;
use alef_core::config::AlefConfig;
use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Package NIF binaries for a single target × all configured NIF versions.
///
/// Returns one `PackageArtifact` per NIF version.
pub fn package_elixir(
    config: &AlefConfig,
    target: &RustTarget,
    workspace_root: &Path,
    output_dir: &Path,
    version: &str,
) -> Result<Vec<PackageArtifact>> {
    let nif_versions = resolve_nif_versions(config);
    let rustler_crate = crate::crate_name_from_output(config, alef_core::config::extras::Language::Elixir)
        .unwrap_or_else(|| config.elixir_app_name().to_lowercase().replace('-', "_") + "_rustler");
    let lib_name = rustler_crate.replace('-', "_");
    let shared_lib = target.shared_lib_name(&lib_name);

    // Locate the built NIF shared library.
    let lib_src = find_elixir_nif(workspace_root, target, &shared_lib)?;

    let ext = nif_extension(target);

    let mut artifacts = Vec::new();
    for nif_version in &nif_versions {
        let tarball_name = format!(
            "lib{lib_name}-v{version}-nif-{nif_version}-{triple}.{ext}.tar.gz",
            triple = target.triple,
        );
        let tarball_path = output_dir.join(&tarball_name);

        // Create a temporary staging dir with the .so/.dll renamed to the
        // RustlerPrecompiled convention.
        let stage_dir = output_dir.join(format!("_stage_{lib_name}_{nif_version}"));
        if stage_dir.exists() {
            fs::remove_dir_all(&stage_dir)?;
        }
        fs::create_dir_all(&stage_dir)?;

        let staged_name = format!("lib{lib_name}.{ext}");
        fs::copy(&lib_src, stage_dir.join(&staged_name))?;

        // Pack as tar.gz.
        super::create_tar_gz(&stage_dir, &tarball_path)
            .with_context(|| format!("creating tarball {}", tarball_path.display()))?;

        let _ = fs::remove_dir_all(&stage_dir);

        artifacts.push(PackageArtifact {
            path: tarball_path,
            name: tarball_name,
            checksum: None,
        });
    }

    Ok(artifacts)
}

/// Generate a `checksum-Elixir.{App}.exs` file from all `.tar.gz` files in `output_dir`.
///
/// Walks `output_dir` for files matching `lib{app}*nif*.tar.gz`, computes SHA256 for each,
/// and writes an Elixir map literal compatible with RustlerPrecompiled.
pub fn write_elixir_checksums(config: &AlefConfig, output_dir: &Path) -> Result<PathBuf> {
    let app_name = config.elixir_app_name();
    // Elixir module name convention: capitalise first letter.
    let module_name = {
        let mut chars = app_name.chars();
        chars.next().map(|c| c.to_uppercase().to_string()).unwrap_or_default() + chars.as_str()
    };

    // Find all NIF tarballs.
    let mut checksums: BTreeMap<String, String> = BTreeMap::new();
    for entry in fs::read_dir(output_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if !name.ends_with(".tar.gz") || !name.contains("-nif-") {
            continue;
        }
        let digest = sha256_file(&path)?;
        checksums.insert(name.to_string(), format!("sha256:{digest}"));
    }

    // Write checksum file.
    let pkg_dir = config.package_dir(alef_core::config::extras::Language::Elixir);
    let checksum_path = Path::new(&pkg_dir).join(format!("checksum-Elixir.{module_name}.Native.exs"));
    let mut content = String::from("%{\n");
    for (file, digest) in &checksums {
        content.push_str(&format!("  \"{file}\" => \"{digest}\",\n"));
    }
    content.push_str("}\n");
    fs::create_dir_all(checksum_path.parent().unwrap_or(Path::new(".")))?;
    fs::write(&checksum_path, content)?;

    Ok(checksum_path)
}

/// Return the native extension suffix for RustlerPrecompiled filenames.
fn nif_extension(target: &RustTarget) -> &'static str {
    match target.os {
        crate::platform::Os::Windows => "dll",
        _ => "so",
    }
}

fn resolve_nif_versions(config: &AlefConfig) -> Vec<String> {
    if let Some(publish) = &config.publish {
        if let Some(lang_cfg) = publish.languages.get("elixir") {
            if let Some(versions) = &lang_cfg.nif_versions {
                if !versions.is_empty() {
                    return versions.clone();
                }
            }
        }
    }
    // Sensible defaults matching typical RustlerPrecompiled setups.
    vec!["2.16".to_string(), "2.17".to_string()]
}

fn find_elixir_nif(workspace_root: &Path, target: &RustTarget, shared_lib: &str) -> Result<PathBuf> {
    let cross = workspace_root
        .join("target")
        .join(&target.triple)
        .join("release")
        .join(shared_lib);
    if cross.exists() {
        return Ok(cross);
    }
    let native = workspace_root.join("target/release").join(shared_lib);
    if native.exists() {
        return Ok(native);
    }
    anyhow::bail!("Elixir NIF '{shared_lib}' not found for target {}", target.triple)
}

/// Compute SHA-256 hex digest of a file.
fn sha256_file(path: &Path) -> Result<String> {
    use std::io::Read;
    let mut file = fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize_hex())
}

/// Minimal SHA-256 implementation to avoid adding a dependency.
struct Sha256 {
    state: [u32; 8],
    buf: Vec<u8>,
}

impl Sha256 {
    fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
            ],
            buf: Vec::new(),
        }
    }

    fn update(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    fn finalize_hex(mut self) -> String {
        // Padding.
        let orig_len_bits = (self.buf.len() as u64) * 8;
        self.buf.push(0x80);
        while self.buf.len() % 64 != 56 {
            self.buf.push(0);
        }
        self.buf.extend_from_slice(&orig_len_bits.to_be_bytes());

        // Process blocks.
        let k: [u32; 64] = SHA256_K;
        for block in self.buf.chunks_exact(64) {
            let mut w = [0u32; 64];
            for (i, chunk) in block.chunks_exact(4).enumerate().take(16) {
                w[i] = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            }
            for i in 16..64 {
                let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
                let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
                w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
            }
            let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
            for i in 0..64 {
                let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
                let ch = (e & f) ^ (!e & g);
                let temp1 = h
                    .wrapping_add(s1)
                    .wrapping_add(ch)
                    .wrapping_add(k[i])
                    .wrapping_add(w[i]);
                let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
                let maj = (a & b) ^ (a & c) ^ (b & c);
                let temp2 = s0.wrapping_add(maj);
                h = g;
                g = f;
                f = e;
                e = d.wrapping_add(temp1);
                d = c;
                c = b;
                b = a;
                a = temp1.wrapping_add(temp2);
            }
            self.state[0] = self.state[0].wrapping_add(a);
            self.state[1] = self.state[1].wrapping_add(b);
            self.state[2] = self.state[2].wrapping_add(c);
            self.state[3] = self.state[3].wrapping_add(d);
            self.state[4] = self.state[4].wrapping_add(e);
            self.state[5] = self.state[5].wrapping_add(f);
            self.state[6] = self.state[6].wrapping_add(g);
            self.state[7] = self.state[7].wrapping_add(h);
        }
        self.state
            .iter()
            .flat_map(|&w| w.to_be_bytes())
            .map(|b| format!("{b:02x}"))
            .collect()
    }
}

#[allow(clippy::unreadable_literal)]
const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98,
    0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
    0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8,
    0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
    0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819,
    0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
    0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
    0xc67178f2,
];

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn sha256_known_vector() {
        // SHA-256 of empty string.
        let mut h = Sha256::new();
        h.update(b"");
        let hex = h.finalize_hex();
        assert_eq!(hex, "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    }

    #[test]
    fn sha256_hello() {
        let mut h = Sha256::new();
        h.update(b"hello");
        let hex = h.finalize_hex();
        assert_eq!(hex, "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824");
    }

    #[test]
    fn nif_extension_linux() {
        let t = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();
        assert_eq!(nif_extension(&t), "so");
    }

    #[test]
    fn nif_extension_windows() {
        let t = RustTarget::parse("x86_64-pc-windows-msvc").unwrap();
        assert_eq!(nif_extension(&t), "dll");
    }

    #[test]
    fn resolve_nif_versions_defaults() {
        let config: AlefConfig = toml::from_str(
            r#"
languages = ["elixir"]
[crate]
name = "my-lib"
sources = ["src/lib.rs"]
"#,
        )
        .unwrap();
        let versions = resolve_nif_versions(&config);
        assert!(!versions.is_empty());
    }

    #[test]
    fn write_checksums_produces_exs_file() {
        let tmp = TempDir::new().unwrap();
        let config: AlefConfig = toml::from_str(&format!(
            r#"
languages = ["elixir"]
[crate]
name = "mylib"
sources = ["src/lib.rs"]
[elixir]
scaffold_output = "{pkg}"
"#,
            pkg = tmp.path().display()
        ))
        .unwrap();

        // Create a fake tarball.
        let tarball = tmp
            .path()
            .join("libmylib-v1.0.0-nif-2.16-x86_64-unknown-linux-gnu.so.tar.gz");
        fs::write(&tarball, b"fake tarball content").unwrap();

        let result = write_elixir_checksums(&config, tmp.path());
        assert!(result.is_ok(), "{result:?}");
        let checksum_file = result.unwrap();
        assert!(checksum_file.exists());
        let content = fs::read_to_string(&checksum_file).unwrap();
        assert!(content.contains("sha256:"));
        assert!(content.contains("nif-2.16"));
    }
}
