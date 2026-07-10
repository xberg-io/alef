/// Backend that emits the Rust JNI shim crate source.
#[derive(Debug, Default, Clone, Copy)]
pub struct JniBackend;

impl Backend for JniBackend {
    fn name(&self) -> &str {
        "jni"
    }

    fn language(&self) -> Language {
        Language::Jni
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            supports_async: true,
            supports_classes: true,
            supports_enums: false,
            supports_option: true,
            supports_result: true,
            supports_callbacks: false,
            supports_streaming: true,
            supports_service_api: true,
        }
    }

    fn generate_bindings(&self, api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        if config.kotlin_android.is_none() {
            anyhow::bail!(
                "kotlin-android config required for JNI shim generation: \
                 add [crates.kotlin_android] with package = \"...\" to alef.toml"
            );
        }
        let output_path = jni_output_path(config);
        let content = emit_lib_rs(api, config);
        Ok(vec![GeneratedFile {
            path: output_path,
            content,
            generated_header: true,
        }])
    }

    fn generate_service_api(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        super::service_api::generate(api, config)
    }

    fn build_config(&self) -> Option<BuildConfig> {
        Some(BuildConfig {
            tool: "cargo",
            crate_suffix: "-jni",
            build_dep: BuildDependency::Ffi,
            post_build: vec![],
        })
    }
}

/// Default output directory: `crates/<crate-base>-jni/src/lib.rs`
///
/// `crate-base` is `config.jni_crate_base()`: `[crates.jni] crate_dir` when
/// set, otherwise `config.name`.  The override lets consumers whose name
/// carries a language suffix (e.g. `"sample-markdown-rs"`) produce a crate
/// at `crates/sample-markdown-jni/` that matches all other binding crates.
fn jni_output_path(config: &ResolvedCrateConfig) -> PathBuf {
    let jni_crate = format!("{}-jni", config.jni_crate_base());
    PathBuf::from(format!("crates/{jni_crate}/src/lib.rs"))
}
