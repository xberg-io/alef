//! Rust target triple parsing and per-language platform name mapping.

use alef_core::config::extras::Language;
use anyhow::{Result, bail};
use std::fmt;

/// CPU architecture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Arch {
    X86_64,
    Aarch64,
    Arm,
    Wasm32,
}

impl fmt::Display for Arch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Arch::X86_64 => write!(f, "x86_64"),
            Arch::Aarch64 => write!(f, "aarch64"),
            Arch::Arm => write!(f, "arm"),
            Arch::Wasm32 => write!(f, "wasm32"),
        }
    }
}

/// Operating system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Os {
    Linux,
    MacOs,
    Windows,
    Unknown,
}

impl fmt::Display for Os {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Os::Linux => write!(f, "linux"),
            Os::MacOs => write!(f, "macos"),
            Os::Windows => write!(f, "windows"),
            Os::Unknown => write!(f, "unknown"),
        }
    }
}

/// C runtime / ABI environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Env {
    Gnu,
    Musl,
    Msvc,
    GnuEabihf,
    None,
}

/// A parsed Rust target triple with helpers for per-language platform naming.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RustTarget {
    /// The original Rust target triple (e.g. `x86_64-unknown-linux-gnu`).
    pub triple: String,
    pub arch: Arch,
    pub os: Os,
    pub env: Env,
}

impl RustTarget {
    /// Parse a Rust target triple string.
    pub fn parse(triple: &str) -> Result<Self> {
        let parts: Vec<&str> = triple.split('-').collect();
        if parts.len() < 2 {
            bail!("invalid target triple: {triple}");
        }

        let arch = match parts[0] {
            "x86_64" => Arch::X86_64,
            "aarch64" => Arch::Aarch64,
            "arm" | "armv7" => Arch::Arm,
            "wasm32" => Arch::Wasm32,
            other => bail!("unsupported architecture: {other}"),
        };

        let os = if triple.contains("linux") {
            Os::Linux
        } else if triple.contains("apple") || triple.contains("darwin") {
            Os::MacOs
        } else if triple.contains("windows") || triple.contains("pc-windows") {
            Os::Windows
        } else if triple.contains("wasm") {
            Os::Unknown
        } else {
            bail!("unsupported OS in target triple: {triple}");
        };

        let env = if triple.contains("gnueabihf") {
            Env::GnuEabihf
        } else if triple.contains("musl") {
            Env::Musl
        } else if triple.contains("gnu") {
            Env::Gnu
        } else if triple.contains("msvc") {
            Env::Msvc
        } else {
            Env::None
        };

        Ok(Self {
            triple: triple.to_string(),
            arch,
            os,
            env,
        })
    }

    /// Return the platform string for a given language.
    ///
    /// Each language ecosystem uses different platform naming conventions.
    /// This method maps the Rust target triple to the correct convention.
    pub fn platform_for(&self, lang: Language) -> String {
        match lang {
            Language::Go | Language::Java => self.go_java_platform(),
            Language::Csharp => self.csharp_rid(),
            Language::Node => self.node_platform(),
            Language::Ruby => self.ruby_platform(),
            Language::Elixir | Language::Ffi | Language::Rust => self.triple.clone(),
            Language::Python => self.python_platform(),
            Language::Php => self.go_java_platform(),
            Language::Wasm => "wasm32".to_string(),
            Language::R => self.triple.clone(),
            Language::Kotlin | Language::Swift | Language::Dart | Language::Gleam | Language::Zig => {
                self.triple.clone()
            }
        }
    }

    /// Go / Java / PHP platform label (e.g. `linux-x86_64`, `macos-arm64`).
    fn go_java_platform(&self) -> String {
        let os = match self.os {
            Os::Linux => "linux",
            Os::MacOs => "macos",
            Os::Windows => "windows",
            Os::Unknown => "unknown",
        };
        // Go uses arm64, not aarch64 for macOS ARM
        let arch = match (self.os, self.arch) {
            (Os::MacOs, Arch::Aarch64) => "arm64",
            (_, Arch::X86_64) => "x86_64",
            (_, Arch::Aarch64) => "aarch64",
            (_, Arch::Arm) => "arm",
            (_, Arch::Wasm32) => "wasm32",
        };
        let suffix = match self.env {
            Env::Musl => "-musl",
            _ => "",
        };
        format!("{os}-{arch}{suffix}")
    }

    /// C# Runtime Identifier (e.g. `linux-x64`, `osx-arm64`, `win-x64`).
    fn csharp_rid(&self) -> String {
        let os = match self.os {
            Os::Linux => "linux",
            Os::MacOs => "osx",
            Os::Windows => "win",
            Os::Unknown => "unknown",
        };
        let arch = match self.arch {
            Arch::X86_64 => "x64",
            Arch::Aarch64 => "arm64",
            Arch::Arm => "arm",
            Arch::Wasm32 => "wasm32",
        };
        let suffix = match self.env {
            Env::Musl if self.os == Os::Linux => format!("-musl-{arch}"),
            _ => format!("-{arch}"),
        };
        format!("{os}{suffix}")
    }

    /// Node / npm platform label (e.g. `linux-x64-gnu`, `darwin-arm64`, `win32-x64-msvc`).
    fn node_platform(&self) -> String {
        let os = match self.os {
            Os::Linux => "linux",
            Os::MacOs => "darwin",
            Os::Windows => "win32",
            Os::Unknown => "unknown",
        };
        let arch = match self.arch {
            Arch::X86_64 => "x64",
            Arch::Aarch64 => "arm64",
            Arch::Arm => "arm",
            Arch::Wasm32 => "wasm32",
        };
        let env = match self.env {
            Env::Gnu => "-gnu",
            Env::Musl => "-musl",
            Env::Msvc => "-msvc",
            Env::GnuEabihf => "-gnueabihf",
            Env::None => "",
        };
        format!("{os}-{arch}{env}")
    }

    /// Ruby platform label (e.g. `x86_64-linux`, `arm64-darwin`).
    fn ruby_platform(&self) -> String {
        let arch = match self.arch {
            Arch::X86_64 => "x86_64",
            Arch::Aarch64 => "aarch64",
            Arch::Arm => "arm",
            Arch::Wasm32 => "wasm32",
        };
        let os = match self.os {
            Os::Linux => "linux",
            Os::MacOs => "darwin",
            Os::Windows => "mingw-ucrt",
            Os::Unknown => "unknown",
        };
        // Ruby uses arm64-darwin instead of aarch64-darwin
        let arch_display = if self.arch == Arch::Aarch64 && self.os == Os::MacOs {
            "arm64"
        } else {
            arch
        };
        let suffix = match self.env {
            Env::Musl if self.os == Os::Linux => "-musl",
            _ => "",
        };
        format!("{arch_display}-{os}{suffix}")
    }

    /// Python platform tag fragment (e.g. `linux-x86_64`, `macos-arm64`).
    fn python_platform(&self) -> String {
        self.go_java_platform()
    }

    /// Return the shared library filename for an FFI crate on this target.
    pub fn shared_lib_name(&self, lib_name: &str) -> String {
        match self.os {
            Os::Linux => format!("lib{lib_name}.so"),
            Os::MacOs => format!("lib{lib_name}.dylib"),
            Os::Windows => format!("{lib_name}.dll"),
            Os::Unknown => format!("lib{lib_name}.so"),
        }
    }

    /// Return the static library filename for this target.
    pub fn static_lib_name(&self, lib_name: &str) -> String {
        match self.os {
            Os::Windows => format!("{lib_name}.lib"),
            _ => format!("lib{lib_name}.a"),
        }
    }

    /// Return the appropriate archive extension for this target.
    pub fn archive_ext(&self) -> &str {
        match self.os {
            Os::Windows => "zip",
            _ => "tar.gz",
        }
    }

    /// Return the binary extension for this target.
    pub fn binary_ext(&self) -> &str {
        match self.os {
            Os::Windows => ".exe",
            _ => "",
        }
    }
}

impl fmt::Display for RustTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.triple)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_linux_gnu() {
        let t = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();
        assert_eq!(t.arch, Arch::X86_64);
        assert_eq!(t.os, Os::Linux);
        assert_eq!(t.env, Env::Gnu);
    }

    #[test]
    fn parse_darwin() {
        let t = RustTarget::parse("aarch64-apple-darwin").unwrap();
        assert_eq!(t.arch, Arch::Aarch64);
        assert_eq!(t.os, Os::MacOs);
        assert_eq!(t.env, Env::None);
    }

    #[test]
    fn parse_windows_msvc() {
        let t = RustTarget::parse("x86_64-pc-windows-msvc").unwrap();
        assert_eq!(t.arch, Arch::X86_64);
        assert_eq!(t.os, Os::Windows);
        assert_eq!(t.env, Env::Msvc);
    }

    #[test]
    fn parse_musl() {
        let t = RustTarget::parse("x86_64-unknown-linux-musl").unwrap();
        assert_eq!(t.arch, Arch::X86_64);
        assert_eq!(t.os, Os::Linux);
        assert_eq!(t.env, Env::Musl);
    }

    #[test]
    fn parse_arm_gnueabihf() {
        let t = RustTarget::parse("arm-unknown-linux-gnueabihf").unwrap();
        assert_eq!(t.arch, Arch::Arm);
        assert_eq!(t.os, Os::Linux);
        assert_eq!(t.env, Env::GnuEabihf);
    }

    #[test]
    fn parse_wasm() {
        let t = RustTarget::parse("wasm32-unknown-unknown").unwrap();
        assert_eq!(t.arch, Arch::Wasm32);
        assert_eq!(t.os, Os::Unknown);
    }

    #[test]
    fn parse_invalid() {
        assert!(RustTarget::parse("invalid").is_err());
    }

    // Platform mapping tests

    #[test]
    fn go_java_platform_linux_x86() {
        let t = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();
        assert_eq!(t.platform_for(Language::Go), "linux-x86_64");
        assert_eq!(t.platform_for(Language::Java), "linux-x86_64");
    }

    #[test]
    fn go_java_platform_macos_arm64() {
        let t = RustTarget::parse("aarch64-apple-darwin").unwrap();
        assert_eq!(t.platform_for(Language::Go), "macos-arm64");
        assert_eq!(t.platform_for(Language::Java), "macos-arm64");
    }

    #[test]
    fn go_java_platform_windows() {
        let t = RustTarget::parse("x86_64-pc-windows-msvc").unwrap();
        assert_eq!(t.platform_for(Language::Go), "windows-x86_64");
    }

    #[test]
    fn csharp_rid_linux_x64() {
        let t = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();
        assert_eq!(t.platform_for(Language::Csharp), "linux-x64");
    }

    #[test]
    fn csharp_rid_osx_arm64() {
        let t = RustTarget::parse("aarch64-apple-darwin").unwrap();
        assert_eq!(t.platform_for(Language::Csharp), "osx-arm64");
    }

    #[test]
    fn csharp_rid_win_x64() {
        let t = RustTarget::parse("x86_64-pc-windows-msvc").unwrap();
        assert_eq!(t.platform_for(Language::Csharp), "win-x64");
    }

    #[test]
    fn node_platform_linux_x64_gnu() {
        let t = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();
        assert_eq!(t.platform_for(Language::Node), "linux-x64-gnu");
    }

    #[test]
    fn node_platform_darwin_arm64() {
        let t = RustTarget::parse("aarch64-apple-darwin").unwrap();
        assert_eq!(t.platform_for(Language::Node), "darwin-arm64");
    }

    #[test]
    fn node_platform_win32_x64_msvc() {
        let t = RustTarget::parse("x86_64-pc-windows-msvc").unwrap();
        assert_eq!(t.platform_for(Language::Node), "win32-x64-msvc");
    }

    #[test]
    fn node_platform_linux_musl() {
        let t = RustTarget::parse("x86_64-unknown-linux-musl").unwrap();
        assert_eq!(t.platform_for(Language::Node), "linux-x64-musl");
    }

    #[test]
    fn ruby_platform_x86_64_linux() {
        let t = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();
        assert_eq!(t.platform_for(Language::Ruby), "x86_64-linux");
    }

    #[test]
    fn ruby_platform_arm64_darwin() {
        let t = RustTarget::parse("aarch64-apple-darwin").unwrap();
        assert_eq!(t.platform_for(Language::Ruby), "arm64-darwin");
    }

    #[test]
    fn ruby_platform_aarch64_linux() {
        let t = RustTarget::parse("aarch64-unknown-linux-gnu").unwrap();
        assert_eq!(t.platform_for(Language::Ruby), "aarch64-linux");
    }

    #[test]
    fn elixir_uses_rust_triple() {
        let t = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();
        assert_eq!(t.platform_for(Language::Elixir), "x86_64-unknown-linux-gnu");
    }

    #[test]
    fn ffi_uses_rust_triple() {
        let t = RustTarget::parse("aarch64-apple-darwin").unwrap();
        assert_eq!(t.platform_for(Language::Ffi), "aarch64-apple-darwin");
    }

    // Library naming tests

    #[test]
    fn shared_lib_linux() {
        let t = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();
        assert_eq!(t.shared_lib_name("html_to_markdown_ffi"), "libhtml_to_markdown_ffi.so");
    }

    #[test]
    fn shared_lib_macos() {
        let t = RustTarget::parse("aarch64-apple-darwin").unwrap();
        assert_eq!(
            t.shared_lib_name("html_to_markdown_ffi"),
            "libhtml_to_markdown_ffi.dylib"
        );
    }

    #[test]
    fn shared_lib_windows() {
        let t = RustTarget::parse("x86_64-pc-windows-msvc").unwrap();
        assert_eq!(t.shared_lib_name("html_to_markdown_ffi"), "html_to_markdown_ffi.dll");
    }

    #[test]
    fn static_lib_unix() {
        let t = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();
        assert_eq!(t.static_lib_name("html_to_markdown_ffi"), "libhtml_to_markdown_ffi.a");
    }

    #[test]
    fn static_lib_windows() {
        let t = RustTarget::parse("x86_64-pc-windows-msvc").unwrap();
        assert_eq!(t.static_lib_name("html_to_markdown_ffi"), "html_to_markdown_ffi.lib");
    }

    #[test]
    fn archive_ext_unix() {
        let t = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();
        assert_eq!(t.archive_ext(), "tar.gz");
    }

    #[test]
    fn archive_ext_windows() {
        let t = RustTarget::parse("x86_64-pc-windows-msvc").unwrap();
        assert_eq!(t.archive_ext(), "zip");
    }

    #[test]
    fn binary_ext_unix() {
        let t = RustTarget::parse("aarch64-apple-darwin").unwrap();
        assert_eq!(t.binary_ext(), "");
    }

    #[test]
    fn binary_ext_windows() {
        let t = RustTarget::parse("x86_64-pc-windows-msvc").unwrap();
        assert_eq!(t.binary_ext(), ".exe");
    }
}
