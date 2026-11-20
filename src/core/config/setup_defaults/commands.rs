use super::*;

pub(super) fn rust_config(lang: Language, ctx: &LangContext) -> SetupConfig {
    {
        let mut commands: Vec<String> = vec!["rustup update stable".to_string()];
        commands.extend(
            ctx.tools
                .rust_tools()
                .iter()
                .map(|t| format!("cargo install {t} --locked")),
        );
        commands.push("rustup component add rustfmt clippy".to_string());
        SetupConfig {
            precondition: Some(require_tool("cargo")),
            before: None,
            install: Some(StringOrVec::Multiple(commands)),
            timeout_seconds: 1800,
            workdir: default_setup_workdir(lang),
        }
    }
}

pub(super) fn python_config(lang: Language, output_dir: &str, ctx: &LangContext) -> SetupConfig {
    {
        let pm = ctx.tools.python_pm();
        let install_cmd = match pm {
            "pip" => format!("cd {output_dir} && pip install -e ."),
            "poetry" => format!("cd {output_dir} && poetry install"),
            _ => format!("cd {output_dir} && uv sync --no-install-project --no-install-workspace"),
        };
        SetupConfig {
            precondition: Some(require_tool(pm)),
            before: None,
            install: Some(StringOrVec::Single(install_cmd)),
            timeout_seconds: 1800,
            workdir: default_setup_workdir(lang),
        }
    }
}

pub(super) fn node_wasm_config(lang: Language, output_dir: &str, ctx: &LangContext) -> SetupConfig {
    {
        let pm = ctx.tools.node_pm();
        let install_cmd = match pm {
            "npm" => format!("cd {output_dir} && npm install"),
            "yarn" => format!("cd {output_dir} && yarn install"),
            _ => format!("cd {output_dir} && pnpm install --config.minimumReleaseAge=0"),
        };
        SetupConfig {
            precondition: Some(require_tool(pm)),
            before: None,
            install: Some(StringOrVec::Single(install_cmd)),
            timeout_seconds: 1800,
            workdir: default_setup_workdir(lang),
        }
    }
}

pub(super) fn go_config(lang: Language, output_dir: &str) -> SetupConfig {
    SetupConfig {
        precondition: Some(require_tool("go")),
        before: None,
        install: Some(StringOrVec::Single(format!(
            "cd {output_dir} && GOWORK=off go mod download"
        ))),
        timeout_seconds: 1800,
        workdir: default_setup_workdir(lang),
    }
}

pub(super) fn ruby_config(lang: Language, output_dir: &str) -> SetupConfig {
    SetupConfig {
        precondition: Some(require_ruby_bundler()),
        before: None,
        install: Some(StringOrVec::Multiple(vec![
            format!("cd {output_dir} && {}", ruby_bundle("install")),
            format!(
                "cd {output_dir} && {} 2>/dev/null || true",
                ruby_bundle("lock --add-checksums")
            ),
        ])),
        // 1800s is the ceiling for every other language's setup step, but `bundle install`
        // has to compile native extensions for any gem that ships one (nokogiri, grpc, ffi-
        // backed gems, ...), and 0.82.0 removed `[setup.ruby].timeout_seconds` as an escape
        // hatch a consumer could raise for a slow CI runner or a cold gem cache -- a real
        // consumer repo's Ruby setup genuinely needed 3000s once that escape hatch was the
        // only thing keeping it from timing out at 1800s. This is a fact about `bundle
        // install` under those conditions, not a per-repo preference, so it belongs in the
        // default rather than requiring every Ruby-targeting repo to still know to raise it.
        // ~keep
        timeout_seconds: 3000,
        workdir: default_setup_workdir(lang),
    }
}

pub(super) fn php_config(lang: Language, output_dir: &str) -> SetupConfig {
    SetupConfig {
        precondition: Some(require_tool("composer")),
        before: None,
        // Some repos keep composer.json only at the repository root; Composer would otherwise
        // prompt interactively to use the parent directory and hang until the timeout.
        install: Some(StringOrVec::Single(format!(
            "if [ -f {output_dir}/composer.json ]; then cd {output_dir}; fi && composer install --no-interaction"
        ))),
        timeout_seconds: 1800,
        workdir: default_setup_workdir(lang),
    }
}

pub(super) fn java_config(lang: Language, output_dir: &str) -> SetupConfig {
    SetupConfig {
        precondition: Some(require_tool("mvn")),
        before: None,
        install: Some(StringOrVec::Single(format!(
            "mvn -f {output_dir}/pom.xml dependency:resolve --batch-mode --no-transfer-progress"
        ))),
        timeout_seconds: 1800,
        workdir: default_setup_workdir(lang),
    }
}

pub(super) fn csharp_config(lang: Language, output_dir: &str) -> SetupConfig {
    SetupConfig {
        precondition: Some(format!(
            "command -v dotnet >/dev/null 2>&1 && [ -n \"$(find {output_dir} -maxdepth 3 \\( -name '*.sln' -o -name '*.csproj' \\) 2>/dev/null | head -1)\" ]"
        )),
        before: None,
        install: Some(StringOrVec::Single(format!(
            "dotnet restore $(find {output_dir} -maxdepth 3 \\( -name '*.sln' -o -name '*.csproj' \\) 2>/dev/null | head -1)"
        ))),
        timeout_seconds: 1800,
        workdir: default_setup_workdir(lang),
    }
}

pub(super) fn elixir_config(lang: Language, output_dir: &str) -> SetupConfig {
    SetupConfig {
        precondition: Some(require_tool("mix")),
        before: None,
        install: Some(StringOrVec::Single(format!("cd {output_dir} && mix deps.get"))),
        timeout_seconds: 1800,
        workdir: default_setup_workdir(lang),
    }
}

pub(super) fn r_config(lang: Language, output_dir: &str) -> SetupConfig {
    SetupConfig {
        precondition: Some(require_tool("Rscript")),
        before: None,
        install: Some(StringOrVec::Single(format!(
            "cd {output_dir} && Rscript -e \"remotes::install_deps()\""
        ))),
        timeout_seconds: 1800,
        workdir: default_setup_workdir(lang),
    }
}

pub(super) fn ffi_config(lang: Language) -> SetupConfig {
    SetupConfig {
        precondition: None,
        before: None,
        install: None,
        timeout_seconds: 1800,
        workdir: default_setup_workdir(lang),
    }
}

pub(super) fn c_config(lang: Language) -> SetupConfig {
    SetupConfig {
        precondition: None,
        before: None,
        install: None,
        timeout_seconds: 1800,
        workdir: default_setup_workdir(lang),
    }
}

pub(super) fn kotlin_kotlin_android_config(lang: Language) -> SetupConfig {
    SetupConfig {
        precondition: Some(require_tool("gradle")),
        before: None,
        install: Some(StringOrVec::Single("gradle --no-daemon tasks --all".to_string())),
        timeout_seconds: 1800,
        workdir: default_setup_workdir(lang),
    }
}

pub(super) fn swift_config(lang: Language) -> SetupConfig {
    SetupConfig {
        precondition: Some(require_tool("swift")),
        before: None,
        install: Some(StringOrVec::Single("swift package resolve".to_string())),
        timeout_seconds: 1800,
        workdir: default_setup_workdir(lang),
    }
}

pub(super) fn dart_config(lang: Language) -> SetupConfig {
    SetupConfig {
        precondition: Some(require_tool("dart")),
        before: None,
        install: Some(StringOrVec::Single("dart pub get".to_string())),
        timeout_seconds: 1800,
        workdir: default_setup_workdir(lang),
    }
}

pub(super) fn zig_config(lang: Language) -> SetupConfig {
    SetupConfig {
        precondition: Some(require_tool("zig")),
        before: None,
        install: Some(StringOrVec::Single("zig build --fetch".to_string())),
        timeout_seconds: 1800,
        workdir: default_setup_workdir(lang),
    }
}

pub(super) fn gleam_config(lang: Language, output_dir: &str) -> SetupConfig {
    SetupConfig {
        precondition: Some(require_tool("gleam")),
        before: None,
        install: Some(StringOrVec::Single(format!("cd {output_dir} && gleam deps download"))),
        timeout_seconds: 1800,
        workdir: default_setup_workdir(lang),
    }
}

pub(super) fn jni_config(lang: Language) -> SetupConfig {
    SetupConfig {
        precondition: None,
        before: None,
        install: None,
        timeout_seconds: 1800,
        workdir: default_setup_workdir(lang),
    }
}
