//! Go/Java/Kotlin/C# identifier methods and reverse-DNS derivations.

use super::ResolvedCrateConfig;
use crate::config::derive::{derive_go_module_from_repo, derive_reverse_dns_package};

impl ResolvedCrateConfig {
    /// Get the GitHub repository URL, returning an error when no source has it set.
    ///
    /// Resolution order:
    /// 1. `[e2e.registry] github_repo`
    /// 2. `[scaffold] repository`
    pub fn try_github_repo(&self) -> Result<String, String> {
        if let Some(e2e) = &self.e2e {
            if let Some(url) = &e2e.registry.github_repo {
                return Ok(url.clone());
            }
        }
        if let Some(url) = self.scaffold.as_ref().and_then(|s| s.repository.as_ref()) {
            return Ok(url.clone());
        }
        Err(format!(
            "no repository URL configured — set `[scaffold] repository = \"...\"` (or `[e2e.registry] github_repo`) for crate `{}`",
            self.name
        ))
    }

    /// Get the GitHub repository URL with a vendor-neutral placeholder fallback.
    pub fn github_repo(&self) -> String {
        self.try_github_repo()
            .unwrap_or_else(|_| format!("https://example.invalid/{}", self.name))
    }

    /// Get the Go module path, returning an error when neither `[go].module`
    /// nor a derivable repository URL is configured.
    pub(crate) fn try_go_module(&self) -> Result<String, String> {
        if let Some(module) = self.go.as_ref().and_then(|g| g.module.as_ref()) {
            return Ok(module.clone());
        }
        if let Ok(repo) = self.try_github_repo() {
            if let Some(module) = derive_go_module_from_repo(&repo) {
                return Ok(module);
            }
        }
        Err(format!(
            "no Go module configured — set `[go] module = \"...\"` or `[scaffold] repository = \"https://<host>/<org>/...\"` for crate `{}`",
            self.name
        ))
    }

    /// Get the Go module path with a vendor-neutral placeholder fallback.
    pub fn go_module(&self) -> String {
        self.try_go_module()
            .unwrap_or_else(|_| format!("example.invalid/{}", self.name))
    }

    /// Get the Java package name, returning an error when neither `[java].package`
    /// nor a derivable repository URL is configured.
    pub(crate) fn try_java_package(&self) -> Result<String, String> {
        if let Some(pkg) = self.java.as_ref().and_then(|j| j.package.as_ref()) {
            return Ok(pkg.clone());
        }
        if let Ok(repo) = self.try_github_repo() {
            if let Some(pkg) = derive_reverse_dns_package(&repo) {
                return Ok(pkg);
            }
        }
        Err(format!(
            "no Java package configured — set `[java] package = \"...\"` or `[scaffold] repository = \"https://<host>/<org>/...\"` for crate `{}`",
            self.name
        ))
    }

    /// Get the Java package name with a vendor-neutral placeholder fallback.
    pub fn java_package(&self) -> String {
        self.try_java_package()
            .unwrap_or_else(|_| "unconfigured.alef".to_string())
    }

    /// Get the Java Maven groupId.
    pub fn java_group_id(&self) -> String {
        self.java_package()
    }

    /// Get the Kotlin package name, returning an error when neither
    /// `[kotlin].package` nor a derivable repository URL is configured.
    pub(crate) fn try_kotlin_package(&self) -> Result<String, String> {
        if let Some(pkg) = self.kotlin.as_ref().and_then(|k| k.package.as_ref()) {
            return Ok(pkg.clone());
        }
        if let Ok(repo) = self.try_github_repo() {
            if let Some(pkg) = derive_reverse_dns_package(&repo) {
                return Ok(pkg);
            }
        }
        Err(format!(
            "no Kotlin package configured — set `[kotlin] package = \"...\"` or `[scaffold] repository = \"https://<host>/<org>/...\"` for crate `{}`",
            self.name
        ))
    }

    /// Get the Kotlin package name with a vendor-neutral placeholder fallback.
    pub fn kotlin_package(&self) -> String {
        self.try_kotlin_package()
            .unwrap_or_else(|_| "unconfigured.alef".to_string())
    }

    /// Get the C# namespace.
    pub fn csharp_namespace(&self) -> String {
        self.csharp
            .as_ref()
            .and_then(|c| c.namespace.as_ref())
            .cloned()
            .unwrap_or_else(|| {
                use heck::ToPascalCase;
                self.name.to_pascal_case()
            })
    }
}

#[cfg(test)]
mod tests {
    use crate::config::new_config::NewAlefConfig;

    fn resolved_one(toml: &str) -> super::super::ResolvedCrateConfig {
        let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    fn with_repo(name: &str, repo: &str) -> super::super::ResolvedCrateConfig {
        resolved_one(&format!(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "{name}"
sources = ["src/lib.rs"]

[crates.scaffold]
repository = "{repo}"
"#
        ))
    }

    #[test]
    fn go_module_derives_from_repo() {
        let r = with_repo("my-lib", "https://github.com/foo/my-lib");
        assert_eq!(r.go_module(), "github.com/foo/my-lib");
    }

    #[test]
    fn go_module_explicit_wins_over_repo() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.scaffold]
repository = "https://github.com/foo/my-lib"

[crates.go]
module = "custom.example.com/my-lib"
"#,
        );
        assert_eq!(r.go_module(), "custom.example.com/my-lib");
    }

    #[test]
    fn java_package_derives_from_repo() {
        let r = with_repo("my-lib", "https://github.com/foo-org/my-lib");
        assert_eq!(r.java_package(), "com.github.foo_org");
    }

    #[test]
    fn java_package_explicit_wins() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.java]
package = "dev.kreuzberg"
"#,
        );
        assert_eq!(r.java_package(), "dev.kreuzberg");
    }

    #[test]
    fn kotlin_package_falls_back_to_placeholder() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]
"#,
        );
        assert_eq!(r.kotlin_package(), "unconfigured.alef");
    }

    #[test]
    fn csharp_namespace_derives_pascal_case() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]
"#,
        );
        assert_eq!(r.csharp_namespace(), "MyLib");
    }

    #[test]
    fn java_group_id_equals_package() {
        let r = with_repo("my-lib", "https://github.com/foo-org/my-lib");
        assert_eq!(r.java_group_id(), r.java_package());
    }
}
