use super::{WriteReport, normalize_content, write};
use crate::core::backend::GeneratedFile;
use crate::core::config::Language;
use std::path::Path;

const MANIFESTS: [&str; 3] = [
    "test_apps/dart/pubspec.yaml",
    "test_apps/elixir/mix.exs",
    "test_apps/kotlin_android/build.gradle.kts",
];

#[derive(Clone, Copy, Debug)]
enum Writer {
    Scaffold,
    Binding,
}

impl Writer {
    fn write(self, base: &Path, relative: &str, version: &str) -> WriteReport {
        let file = GeneratedFile {
            path: relative.into(),
            content: manifest_body(relative, version),
            generated_header: false,
        };
        match self {
            Self::Scaffold => super::scaffold::write_scaffold_files_report(&[file], base, true),
            Self::Binding => write::write_files_report(&[(Language::Rust, vec![file])], base),
        }
        .expect("write manifest seed")
    }
}

fn manifest_body(relative: &str, version: &str) -> String {
    if relative.ends_with(".yaml") {
        format!("version: \"{version}\"\n")
    } else {
        format!("version = \"{version}\"\n")
    }
}

fn seed(base: &Path, relative: &str, adopted: bool) -> String {
    let path = base.join(relative);
    std::fs::create_dir_all(path.parent().expect("manifest parent")).expect("create parent");
    let original = manifest_body(relative, "1.0.0");
    let content = if adopted {
        write::stamp_for_adoption(&path, &original).expect("manifest supports adoption marker")
    } else {
        original
    };
    std::fs::write(path, &content).expect("seed manifest");
    content
}

#[test]
fn adopted_seed_markers_survive_repeated_version_regeneration() {
    for writer in [Writer::Scaffold, Writer::Binding] {
        for relative in MANIFESTS {
            let temporary = tempfile::tempdir().expect("temporary directory");
            let base = temporary.path();
            seed(base, relative, true);
            for version in ["1.0.1", "1.0.2"] {
                let report = writer.write(base, relative, version);
                assert_eq!(report.refused_paths.len(), 0, "{writer:?}: {relative}");
                assert_eq!(report.changed_count(), 1, "{writer:?}: {relative}");
                let path = base.join(relative);
                let actual = std::fs::read_to_string(&path).expect("read updated manifest");
                let body = normalize_content(&path, &manifest_body(relative, version));
                assert_eq!(
                    actual,
                    write::ensure_generated_header(&path, &body),
                    "{writer:?}: {relative} must retain ownership for the next version bump"
                );
            }
        }
    }
}

#[test]
fn new_seeds_remain_unmarked_and_refuse_later_version_changes() {
    for writer in [Writer::Scaffold, Writer::Binding] {
        for relative in MANIFESTS {
            let temporary = tempfile::tempdir().expect("temporary directory");
            let base = temporary.path();
            assert_eq!(writer.write(base, relative, "1.0.0").changed_count(), 1);
            let path = base.join(relative);
            let original = std::fs::read_to_string(&path).expect("read new seed");
            assert_eq!(original, normalize_content(&path, &manifest_body(relative, "1.0.0")));
            let report = writer.write(base, relative, "1.0.1");
            assert_eq!(report.refused_paths.into_iter().collect::<Vec<_>>(), vec![path.clone()]);
            assert_eq!(std::fs::read_to_string(path).expect("read refused seed"), original);
        }
    }
}

#[test]
fn unadopted_existing_seeds_remain_untouched() {
    for writer in [Writer::Scaffold, Writer::Binding] {
        for relative in MANIFESTS {
            let temporary = tempfile::tempdir().expect("temporary directory");
            let base = temporary.path();
            let original = seed(base, relative, false);
            let report = writer.write(base, relative, "1.0.1");
            let path = base.join(relative);
            assert_eq!(report.refused_paths.into_iter().collect::<Vec<_>>(), vec![path.clone()]);
            assert_eq!(std::fs::read_to_string(path).expect("read unadopted seed"), original);
        }
    }
}

#[test]
fn declared_user_owned_seeds_keep_their_original_bytes_even_when_marked() {
    for writer in [Writer::Scaffold, Writer::Binding] {
        for relative in MANIFESTS {
            let temporary = tempfile::tempdir().expect("temporary directory");
            let base = temporary.path();
            let original = seed(base, relative, true);
            std::fs::write(
                base.join("alef.toml"),
                format!(
                    "[workspace.ownership]\nuser_owned = [\"{relative}\"]\n\n\
                     [[crates]]\nname = \"sample_core\"\nsources = [\"src/lib.rs\"]\n"
                ),
            )
            .expect("declare user-owned seed");
            let report = writer.write(base, relative, "1.0.1");
            let path = base.join(relative);
            assert_eq!(
                report.user_owned_paths.into_iter().collect::<Vec<_>>(),
                vec![path.clone()]
            );
            assert_eq!(std::fs::read_to_string(path).expect("read user-owned seed"), original);
        }
    }
}
