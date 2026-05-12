//! Config file generators for TypeScript e2e tests (package.json, tsconfig.json, vitest.config.ts).

use alef_core::hash::{self, CommentStyle};
use minijinja::context;

pub(super) fn render_package_json(
    pkg_name: &str,
    _pkg_path: &str,
    pkg_version: &str,
    dep_mode: crate::config::DependencyMode,
    has_http_fixtures: bool,
) -> String {
    let dep_value = match dep_mode {
        crate::config::DependencyMode::Registry => pkg_version.to_string(),
        crate::config::DependencyMode::Local => "workspace:*".to_string(),
    };
    let _ = has_http_fixtures; // TODO: add HTTP test deps when http fixtures are present

    crate::template_env::render(
        "typescript/package.json.jinja",
        context! {
            pkg_name => pkg_name,
            dep_value => dep_value,
        },
    )
}

pub(super) fn render_tsconfig() -> String {
    crate::template_env::render("typescript/tsconfig.jinja", context! {})
}

pub(super) fn render_vitest_config(with_global_setup: bool, with_file_setup: bool) -> String {
    let header = hash::header(CommentStyle::DoubleSlash);

    crate::template_env::render(
        "typescript/vitest.config.ts.jinja",
        context! {
            header => header,
            with_global_setup => with_global_setup,
            with_file_setup => with_file_setup,
        },
    )
}

pub(super) fn render_file_setup(test_documents_dir: &str) -> String {
    let header = hash::header(CommentStyle::DoubleSlash);

    crate::template_env::render(
        "typescript/setup.ts.jinja",
        context! {
            header => header,
            test_documents_dir => test_documents_dir,
        },
    )
}

pub(super) fn render_global_setup() -> String {
    let header = hash::header(CommentStyle::DoubleSlash);

    crate::template_env::render(
        "typescript/globalSetup.ts.jinja",
        context! {
            header => header,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DependencyMode;

    #[test]
    fn render_package_json_local_uses_workspace_star() {
        let out = render_package_json("my-pkg", "", "1.0.0", DependencyMode::Local, false);
        assert!(out.contains("workspace:*"), "got: {out}");
    }

    #[test]
    fn render_package_json_registry_uses_version() {
        let out = render_package_json("my-pkg", "", "1.2.3", DependencyMode::Registry, false);
        assert!(out.contains("\"1.2.3\""), "got: {out}");
    }

    #[test]
    fn render_vitest_config_with_global_setup_includes_global_setup_key() {
        let out = render_vitest_config(true, false);
        assert!(out.contains("globalSetup"), "got: {out}");
    }

    #[test]
    fn render_vitest_config_without_global_setup_omits_global_setup_key() {
        let out = render_vitest_config(false, false);
        assert!(!out.contains("globalSetup"), "got: {out}");
    }
}
