use crate::core::config::{Language, ResolvedCrateConfig};
use anyhow::Context as _;
use tracing::{debug, info, warn};

/// Build the swift artifactbundle for the current crate, compute its sha256,
/// substitute `__ALEF_SWIFT_CHECKSUM__` in root `Package.swift`, and write a
/// sidecar file at `target/alef-swift-checksum.txt` so the publish workflow can
/// reuse the checksum without rebuilding.
///
/// # Steps
///
/// 1. Detect whether the workspace has a swift binding crate (`{name}-swift`).
///    Skip with a warning if not found.
/// 2. Check whether `Package.swift` still contains `__ALEF_SWIFT_CHECKSUM__`.
///    Return early if already substituted (idempotent).
/// 3. Look for a pre-built `.artifactbundle.zip` under `dist/swift-artifactbundle/`.
///    If none exists, shell out to `cargo build -p {crate}-swift --release` and
///    the alef-bundled build script to produce one.  Skips gracefully when the
///    build prerequisites (Xcode / Apple targets) are absent.
/// 4. Compute the checksum with `swift package compute-checksum {zip}` (falls back
///    to a SHA-256 hex digest computed in-process if `swift` is not on PATH).
/// 5. Substitute the checksum in `Package.swift` and write the sidecar file.
///
/// Returns `Ok(Some(checksum))` when substitution succeeds, `Ok(None)` when skipped.
pub(super) fn precompute_swift_checksum(config: &ResolvedCrateConfig) -> anyhow::Result<Option<String>> {
    use super::helpers::run_command_captured;

    // Guard: Package.swift must exist and still contain the placeholder.
    let pkg_swift_path = std::path::Path::new("Package.swift");
    let pkg_content = match std::fs::read_to_string(pkg_swift_path) {
        Ok(c) => c,
        Err(_) => {
            debug!("Package.swift not found — skipping swift checksum precompute");
            return Ok(None);
        }
    };
    if !pkg_content.contains("__ALEF_SWIFT_CHECKSUM__") {
        debug!("Package.swift already has a real checksum — skipping precompute");
        return Ok(None);
    }

    // Guard: swift must be in the configured languages.
    if !config.languages.contains(&Language::Swift) {
        debug!("Swift not configured — skipping swift checksum precompute");
        return Ok(None);
    }

    // Guard: the swift binding crate must exist. Some consumers put it under
    // `crates/{name}-swift/` (alef default), others under `packages/swift/rust/`.
    // Probe both before giving up.
    let swift_crate = format!("{}-swift", config.name);
    let candidate_manifests = [
        format!("crates/{swift_crate}/Cargo.toml"),
        "packages/swift/rust/Cargo.toml".to_string(),
    ];
    let swift_manifest = candidate_manifests
        .iter()
        .find(|p| std::path::Path::new(p).exists())
        .cloned();
    let Some(swift_manifest) = swift_manifest else {
        warn!(
            "Swift binding crate `{swift_crate}` not found under any of {:?} — \
             skipping checksum precompute. Run with --skip-swift-checksum to suppress.",
            candidate_manifests
        );
        return Ok(None);
    };
    debug!("Using swift manifest: {swift_manifest}");

    // Look for a pre-built artifactbundle zip under dist/swift-artifactbundle/.
    // The build action outputs `{ArtifactName}.artifactbundle.zip` there.
    let bundle_dir = std::path::Path::new("dist/swift-artifactbundle");
    let existing_zip = if bundle_dir.exists() {
        std::fs::read_dir(bundle_dir).ok().and_then(|entries| {
            entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .find(|p| p.extension().and_then(|e| e.to_str()) == Some("zip"))
        })
    } else {
        None
    };

    let zip_path = match existing_zip {
        Some(p) => {
            info!("Using pre-built artifactbundle: {}", p.display());
            p
        }
        None => {
            // No pre-built zip found — attempt to build.
            info!("Building swift artifactbundle for `{swift_crate}`…");
            let build_cmd = format!("cargo build -p {swift_crate} --release --target aarch64-apple-darwin");
            match run_command_captured(&build_cmd) {
                Ok(_) => {}
                Err(e) => {
                    warn!(
                        "Swift artifactbundle build failed (missing Xcode / Apple targets?): {e}\n\
                         Re-run with --skip-swift-checksum to skip this step."
                    );
                    return Ok(None);
                }
            }
            // After cargo build, look again.
            std::fs::create_dir_all(bundle_dir).ok();
            match std::fs::read_dir(bundle_dir).ok().and_then(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .find(|p| p.extension().and_then(|e| e.to_str()) == Some("zip"))
            }) {
                Some(p) => p,
                None => {
                    warn!(
                        "No .zip found in `dist/swift-artifactbundle/` after build — \
                         skipping checksum substitution."
                    );
                    return Ok(None);
                }
            }
        }
    };

    // Compute checksum: prefer `swift package compute-checksum` (canonical tool),
    // fall back to an in-process SHA-256.
    let checksum_cmd = format!("swift package compute-checksum {}", zip_path.display());
    let checksum = match run_command_captured(&checksum_cmd) {
        Ok((stdout, _)) => stdout.trim().to_string(),
        Err(_) => {
            // Fallback: compute SHA-256 in-process.
            info!("`swift` not found — computing SHA-256 in-process");
            let bytes = std::fs::read(&zip_path).with_context(|| format!("failed to read {}", zip_path.display()))?;
            compute_sha256_hex(&bytes)
        }
    };

    if checksum.is_empty() {
        warn!("Computed empty checksum — skipping substitution");
        return Ok(None);
    }

    // Substitute in Package.swift.
    let new_content = pkg_content.replace("__ALEF_SWIFT_CHECKSUM__", &checksum);
    std::fs::write(pkg_swift_path, &new_content).context("writing Package.swift with checksum")?;
    info!("Substituted __ALEF_SWIFT_CHECKSUM__ → {checksum} in Package.swift");

    // Write sidecar so publish.yaml can reuse the hash without rebuilding.
    std::fs::create_dir_all("target").ok();
    std::fs::write("target/alef-swift-checksum.txt", &checksum).context("writing target/alef-swift-checksum.txt")?;

    Ok(Some(checksum))
}

/// Compute a lowercase hex SHA-256 digest of `bytes` without shelling out.
///
/// Used as a fallback when `swift package compute-checksum` is not available.
pub(super) fn compute_sha256_hex(bytes: &[u8]) -> String {
    // sha2 is pulled in transitively (ring → sha2 in some configurations).
    // Use a manual implementation to avoid adding a direct dependency.
    use std::num::Wrapping;

    // SHA-256 round constants K.
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98,
        0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
        0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8,
        0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
        0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819,
        0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
        0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    // Initial hash values H.
    let mut h: [Wrapping<u32>; 8] = [
        Wrapping(0x6a09e667),
        Wrapping(0xbb67ae85),
        Wrapping(0x3c6ef372),
        Wrapping(0xa54ff53a),
        Wrapping(0x510e527f),
        Wrapping(0x9b05688c),
        Wrapping(0x1f83d9ab),
        Wrapping(0x5be0cd19),
    ];

    // Pre-processing: add padding.
    let bit_len = (bytes.len() as u64).wrapping_mul(8);
    let mut msg = bytes.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0x00);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    // Process each 512-bit (64-byte) chunk.
    for chunk in msg.chunks_exact(64) {
        let mut w = [Wrapping(0u32); 64];
        for i in 0..16 {
            w[i] = Wrapping(u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]));
        }
        for i in 16..64 {
            let s0 = w[i - 15].0.rotate_right(7) ^ w[i - 15].0.rotate_right(18) ^ (w[i - 15].0 >> 3);
            let s1 = w[i - 2].0.rotate_right(17) ^ w[i - 2].0.rotate_right(19) ^ (w[i - 2].0 >> 10);
            w[i] = w[i - 16] + Wrapping(s0) + w[i - 7] + Wrapping(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.0.rotate_right(6) ^ e.0.rotate_right(11) ^ e.0.rotate_right(25);
            let ch = (e.0 & f.0) ^ ((!e.0) & g.0);
            let temp1 = hh + Wrapping(s1) + Wrapping(ch) + Wrapping(K[i]) + w[i];
            let s0 = a.0.rotate_right(2) ^ a.0.rotate_right(13) ^ a.0.rotate_right(22);
            let maj = (a.0 & b.0) ^ (a.0 & c.0) ^ (b.0 & c.0);
            let temp2 = Wrapping(s0) + Wrapping(maj);
            hh = g;
            g = f;
            f = e;
            e = d + temp1;
            d = c;
            c = b;
            b = a;
            a = temp1 + temp2;
        }
        h[0] += a;
        h[1] += b;
        h[2] += c;
        h[3] += d;
        h[4] += e;
        h[5] += f;
        h[6] += g;
        h[7] += hh;
    }

    format!(
        "{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}",
        h[0].0, h[1].0, h[2].0, h[3].0, h[4].0, h[5].0, h[6].0, h[7].0
    )
}
