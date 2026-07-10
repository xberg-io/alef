//! Elixir NIF precompiled binary packaging.
//!
//! Produces one tarball per (target × nif_version) combination in the format
//! expected by `RustlerPrecompiled`:
//!
//! `{lib}-v{version}-nif-{nif_version}-{target}.{ext}.tar.gz`
//!
//! where `{ext}` is `dll` (Windows) or `so` (everything else, including macOS).
//! Darwin uses `so` — not `dylib` — to match `rustler_precompiled 0.9.0`'s
//! `lib_name_with_ext/2` consumer-side URL construction, which hardcodes `so`
//! for every non-Windows target and cannot be overridden. No newer
//! `rustler_precompiled` version exists on Hex with `.dylib` support.
//!
//! Also provides `write_elixir_checksums()` to generate the
//! `checksum-Elixir.{App}.exs` file that RustlerPrecompiled validates.

use super::PackageArtifact;
use crate::core::config::ResolvedCrateConfig;
use crate::publish::platform::RustTarget;
use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Package NIF binaries for a single target × all configured NIF versions.
///
/// Returns one `PackageArtifact` per NIF version.
pub fn package_elixir(
    config: &ResolvedCrateConfig,
    target: &RustTarget,
    workspace_root: &Path,
    output_dir: &Path,
    version: &str,
) -> Result<Vec<PackageArtifact>> {
    let nif_versions = resolve_nif_versions(config);
    let rustler_crate = crate::publish::crate_name_from_output(config, crate::core::config::extras::Language::Elixir)
        .unwrap_or_else(|| config.elixir_app_name().to_lowercase().replace('-', "_") + "_rustler");
    let lib_name = rustler_crate.replace('-', "_");
    let shared_lib = target.shared_lib_name(&lib_name);

    let lib_src = find_elixir_nif(workspace_root, target, &shared_lib)?;

    let ext = nif_extension(target);

    let mut artifacts = Vec::new();
    for nif_version in &nif_versions {
        let tarball_name = format!(
            "lib{lib_name}-v{version}-nif-{nif_version}-{triple}.{ext}.tar.gz",
            triple = target.triple,
        );
        let tarball_path = output_dir.join(&tarball_name);

        let stage_dir = output_dir.join(format!("_stage_{lib_name}_{nif_version}"));
        if stage_dir.exists() {
            fs::remove_dir_all(&stage_dir)?;
        }
        fs::create_dir_all(&stage_dir)?;

        let staged_name = format!("lib{lib_name}.{ext}");
        fs::copy(&lib_src, stage_dir.join(&staged_name))?;

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
pub fn write_elixir_checksums(config: &ResolvedCrateConfig, output_dir: &Path) -> Result<PathBuf> {
    let app_name = config.elixir_app_name();
    let module_name = {
        let mut chars = app_name.chars();
        chars.next().map(|c| c.to_uppercase().to_string()).unwrap_or_default() + chars.as_str()
    };

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

    let pkg_dir = config.package_dir(crate::core::config::extras::Language::Elixir);
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
///
/// Returns `dll` for Windows and `so` for every other OS (including macOS).
/// `rustler_precompiled 0.9.0`'s `lib_name_with_ext/2` hardcodes `so` for
/// every non-Windows target when constructing the consumer download URL and
/// cannot be overridden. Publishing `.dylib.tar.gz` for darwin would 404
/// every `mix deps.get` on macOS.
fn nif_extension(target: &RustTarget) -> &'static str {
    match target.os {
        crate::publish::platform::Os::Windows => "dll",
        _ => "so",
    }
}

fn resolve_nif_versions(config: &ResolvedCrateConfig) -> Vec<String> {
    if let Some(publish) = &config.publish {
        if let Some(lang_cfg) = publish.languages.get("elixir") {
            if let Some(versions) = &lang_cfg.nif_versions {
                if !versions.is_empty() {
                    return versions.clone();
                }
            }
        }
    }
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
        let orig_len_bits = (self.buf.len() as u64) * 8;
        self.buf.push(0x80);
        while self.buf.len() % 64 != 56 {
            self.buf.push(0);
        }
        self.buf.extend_from_slice(&orig_len_bits.to_be_bytes());

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

/// SHA-256 round constants (first 32 bits of fractional parts of cube roots of first 64 primes).
const SHA256_K: [u32; 64] = [
    0x428a_2f98,
    0x7137_4491,
    0xb5c0_fbcf,
    0xe9b5_dba5,
    0x3956_c25b,
    0x59f1_11f1,
    0x923f_82a4,
    0xab1c_5ed5,
    0xd807_aa98,
    0x1283_5b01,
    0x2431_85be,
    0x550c_7dc3,
    0x72be_5d74,
    0x80de_b1fe,
    0x9bdc_06a7,
    0xc19b_f174,
    0xe49b_69c1,
    0xefbe_4786,
    0x0fc1_9dc6,
    0x240c_a1cc,
    0x2de9_2c6f,
    0x4a74_84aa,
    0x5cb0_a9dc,
    0x76f9_88da,
    0x983e_5152,
    0xa831_c66d,
    0xb003_27c8,
    0xbf59_7fc7,
    0xc6e0_0bf3,
    0xd5a7_9147,
    0x06ca_6351,
    0x1429_2967,
    0x27b7_0a85,
    0x2e1b_2138,
    0x4d2c_6dfc,
    0x5338_0d13,
    0x650a_7354,
    0x766a_0abb,
    0x81c2_c92e,
    0x9272_2c85,
    0xa2bf_e8a1,
    0xa81a_664b,
    0xc24b_8b70,
    0xc76c_51a3,
    0xd192_e819,
    0xd699_0624,
    0xf40e_3585,
    0x106a_a070,
    0x19a4_c116,
    0x1e37_6c08,
    0x2748_774c,
    0x34b0_bcb5,
    0x391c_0cb3,
    0x4ed8_aa4a,
    0x5b9c_ca4f,
    0x682e_6ff3,
    0x748f_82ee,
    0x78a5_636f,
    0x84c8_7814,
    0x8cc7_0208,
    0x90be_fffa,
    0xa450_6ceb,
    0xbef9_a3f7,
    0xc671_78f2,
];

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn sha256_known_vector() {
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
    fn nif_extension_macos() {
        let t = RustTarget::parse("x86_64-apple-darwin").unwrap();
        assert_eq!(nif_extension(&t), "so");
    }

    #[test]
    fn nif_extension_windows() {
        let t = RustTarget::parse("x86_64-pc-windows-msvc").unwrap();
        assert_eq!(nif_extension(&t), "dll");
    }

    #[test]
    fn resolve_nif_versions_defaults() {
        let cfg: crate::core::config::NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["elixir"]
[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]
"#,
        )
        .unwrap();
        let config = cfg.resolve().unwrap().remove(0);
        let versions = resolve_nif_versions(&config);
        assert!(!versions.is_empty());
    }

    #[test]
    fn write_checksums_produces_exs_file() {
        let tmp = TempDir::new().unwrap();
        let cfg: crate::core::config::NewAlefConfig = toml::from_str(&format!(
            r#"
[workspace]
languages = ["elixir"]
[[crates]]
name = "mylib"
sources = ["src/lib.rs"]
[crates.elixir]
scaffold_output = "{pkg}"
"#,
            pkg = tmp.path().display().to_string().replace('\\', "/")
        ))
        .unwrap();
        let config = cfg.resolve().unwrap().remove(0);

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
