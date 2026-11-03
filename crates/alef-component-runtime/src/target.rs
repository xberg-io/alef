//! Host target detection for native components.

/// The Rust target triple of the host this crate was compiled for, when it is one a component
/// artifact can be published for; `None` otherwise (e.g. mobile, wasm, musl or 32-bit hosts).
///
/// The result is the `identity.target` string used to select a lock entry, so it must agree
/// with the triples `alef component package` records. Generated bindings use the same table.
#[must_use]
#[allow(unreachable_code)]
pub fn host_target() -> Option<&'static str> {
    #[cfg(all(target_arch = "x86_64", target_os = "linux", target_env = "gnu"))]
    return Some("x86_64-unknown-linux-gnu");
    #[cfg(all(target_arch = "aarch64", target_os = "linux", target_env = "gnu"))]
    return Some("aarch64-unknown-linux-gnu");
    #[cfg(all(target_arch = "x86_64", target_os = "macos"))]
    return Some("x86_64-apple-darwin");
    #[cfg(all(target_arch = "aarch64", target_os = "macos"))]
    return Some("aarch64-apple-darwin");
    #[cfg(all(target_arch = "x86_64", target_os = "windows", target_env = "msvc"))]
    return Some("x86_64-pc-windows-msvc");
    #[cfg(all(target_arch = "aarch64", target_os = "windows", target_env = "msvc"))]
    return Some("aarch64-pc-windows-msvc");
    None
}

#[cfg(test)]
mod tests {
    use super::host_target;

    #[test]
    fn host_target_matches_the_compile_time_platform() {
        let Some(triple) = host_target() else {
            return;
        };
        let arch = triple.split('-').next().unwrap();
        assert_eq!(arch, std::env::consts::ARCH);
        let os_marker = match std::env::consts::OS {
            "linux" => "linux",
            "macos" => "apple-darwin",
            "windows" => "windows-msvc",
            other => panic!("host_target returned {triple} on unexpected OS {other}"),
        };
        assert!(triple.ends_with(os_marker) || triple.contains(os_marker), "{triple}");
    }

    #[test]
    fn host_target_is_a_well_formed_triple() {
        if let Some(triple) = host_target() {
            assert!(triple.split('-').count() >= 3, "{triple}");
            assert!(!triple.contains(char::is_whitespace), "{triple}");
        }
    }

    #[test]
    fn supported_linux_and_macos_ci_hosts_are_detected() {
        if cfg!(all(
            target_os = "linux",
            target_env = "gnu",
            any(target_arch = "x86_64", target_arch = "aarch64")
        )) || cfg!(all(
            target_os = "macos",
            any(target_arch = "x86_64", target_arch = "aarch64")
        )) {
            assert!(host_target().is_some());
        }
    }
}
