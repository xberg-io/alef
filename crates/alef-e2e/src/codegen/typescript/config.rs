//! Config file generators for TypeScript e2e tests (package.json, tsconfig.json, vitest.config.ts).

use alef_core::hash::{self, CommentStyle};
use alef_core::template_versions as tv;

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
    format!(
        r#"{{
  "name": "{pkg_name}-e2e",
  "version": "0.1.0",
  "private": true,
  "type": "module",
  "scripts": {{
    "test": "vitest run"
  }},
  "devDependencies": {{
    "{pkg_name}": "{dep_value}",
    "vitest": "{vitest}"
  }}
}}
"#,
        vitest = tv::npm::VITEST,
    )
}

pub(super) fn render_tsconfig() -> String {
    r#"{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ESNext",
    "moduleResolution": "bundler",
    "strict": true,
    "esModuleInterop": true,
    "skipLibCheck": true
  },
  "include": ["tests/**/*.ts", "vitest.config.ts"]
}
"#
    .to_string()
}

pub(super) fn render_vitest_config(with_global_setup: bool, with_file_setup: bool) -> String {
    let header = hash::header(CommentStyle::DoubleSlash);
    let setup_files_line = if with_file_setup {
        "    setupFiles: ['./setup.ts'],\n"
    } else {
        ""
    };
    if with_global_setup {
        format!(
            r#"{header}import {{ defineConfig }} from 'vitest/config';

export default defineConfig({{
  test: {{
    include: ['tests/**/*.test.ts'],
    globalSetup: './globalSetup.ts',
{setup_files_line}  }},
}});
"#
        )
    } else {
        format!(
            r#"{header}import {{ defineConfig }} from 'vitest/config';

export default defineConfig({{
  test: {{
    include: ['tests/**/*.test.ts'],
{setup_files_line}  }},
}});
"#
        )
    }
}

pub(super) fn render_file_setup() -> String {
    let header = hash::header(CommentStyle::DoubleSlash);
    header
        + r#"import { fileURLToPath } from 'url';
import { dirname, join } from 'path';

// Change to the test_documents directory so that fixture file paths like
// "pdf/fake_memo.pdf" resolve correctly when running vitest from e2e/node/.
// setup.ts lives in e2e/node/; test_documents lives at the repository root,
// two directories up: e2e/node/ -> e2e/ -> repo root -> test_documents/.
const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const testDocumentsDir = join(__dirname, '..', '..', 'test_documents');
process.chdir(testDocumentsDir);
"#
}

pub(super) fn render_global_setup() -> String {
    let header = hash::header(CommentStyle::DoubleSlash);
    header
        + r#"import { spawn } from 'child_process';
import { resolve } from 'path';

let serverProcess: any;

// HTTP client wrapper for making requests to mock server
const createApp = (baseUrl: string) => ({
  async request(path: string, init?: RequestInit): Promise<Response> {
    const url = new URL(path, baseUrl);
    return fetch(url.toString(), init);
  },
});

export async function setup() {
  // Mock server binary must be pre-built (e.g. by CI or `cargo build --manifest-path e2e/rust/Cargo.toml --bin mock-server --release`)
  serverProcess = spawn(
    resolve(__dirname, '../rust/target/release/mock-server'),
    [resolve(__dirname, '../../fixtures')],
    { stdio: ['pipe', 'pipe', 'inherit'] }
  );

  const url = await new Promise<string>((resolve, reject) => {
    serverProcess.stdout.on('data', (data: any) => {
      const match = data.toString().match(/MOCK_SERVER_URL=(.*)/);
      if (match) resolve(match[1].trim());
    });
    setTimeout(() => reject(new Error('Mock server startup timeout')), 30000);
  });

  process.env.MOCK_SERVER_URL = url;

  // Make app available globally to all tests
  (globalThis as any).app = createApp(url);
}

export async function teardown() {
  if (serverProcess) {
    serverProcess.stdin.end();
    serverProcess.kill();
  }
}
"#
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
