use alef::core::backend::GeneratedFile;
use alef::core::config::{Language, NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::ApiSurface;

const PYTHON_GLOBS: [&str; 4] = ["**/api.py", "**/*.pyi", "**/options.py", "**/__init__.py"];

fn config() -> ResolvedCrateConfig {
    let raw: NewAlefConfig = toml::from_str(
        "[workspace]\nlanguages=[\"python\",\"dart\"]\n[[crates]]\nname=\"demo\"\nsources=[\"src/lib.rs\"]\n",
    )
    .unwrap();
    raw.resolve().unwrap().remove(0)
}

fn poly(config: &ResolvedCrateConfig, languages: &[Language]) -> GeneratedFile {
    let api = ApiSurface {
        crate_name: "demo".into(),
        ..Default::default()
    };
    alef::scaffold::scaffold(&api, config, languages)
        .unwrap()
        .into_iter()
        .find(|file| file.path.to_str() == Some("poly.toml"))
        .unwrap()
}

fn write_poly(root: &std::path::Path, config: &ResolvedCrateConfig, languages: &[Language]) -> toml::Value {
    let report = alef::cli::pipeline::write_scaffold_files_report(&[poly(config, languages)], root, true).unwrap();
    assert!(report.changed_paths.contains(&root.join("poly.toml")));
    toml::from_str(&std::fs::read_to_string(root.join("poly.toml")).unwrap()).unwrap()
}

#[test]
fn scoped_generation_preserves_configured_python_rules_and_prunes_withdrawn_suppression() {
    let root = tempfile::tempdir().unwrap();
    let mut config = config();
    config
        .poly
        .per_file_ignores
        .insert("paid/off.rs".into(), vec!["function-too-long".into()]);
    let before = write_poly(root.path(), &config, &config.languages);
    for glob in PYTHON_GLOBS {
        assert!(
            before["per-file-ignores"][glob]
                .as_array()
                .is_some_and(|rules| !rules.is_empty())
        );
    }
    assert!(before["per-file-ignores"].get("paid/off.rs").is_some());
    config.poly.per_file_ignores.clear();
    let after = write_poly(root.path(), &config, &[Language::Dart]);
    for glob in PYTHON_GLOBS {
        assert_eq!(
            after["per-file-ignores"].get(glob),
            before["per-file-ignores"].get(glob),
            "lost {glob}"
        );
    }
    assert!(after["per-file-ignores"].get("paid/off.rs").is_none());
    assert_eq!(after["lint"]["python"], before["lint"]["python"]);
}

#[test]
fn removing_python_from_configuration_withdraws_python_suppressions() {
    let root = tempfile::tempdir().unwrap();
    let mut config = config();
    let before = write_poly(root.path(), &config, &config.languages);
    assert!(
        PYTHON_GLOBS
            .iter()
            .all(|glob| before["per-file-ignores"].get(glob).is_some())
    );
    config.languages = vec![Language::Dart];
    let generated = poly(&config, &[Language::Dart]);
    assert!(!generated.content.contains("[lint.python.ruff]"));
    assert!(!generated.content.contains("pyrefly"));
    let after = write_poly(root.path(), &config, &[Language::Dart]);
    for glob in PYTHON_GLOBS {
        assert!(after["per-file-ignores"].get(glob).is_none(), "retained {glob}");
    }
}
