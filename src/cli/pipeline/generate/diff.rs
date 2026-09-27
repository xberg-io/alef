use super::normalization::{format_rust_content, normalize_content, normalize_whitespace};
use crate::core::backend::GeneratedFile;
use crate::core::config::Language;
use crate::core::hash;
use rayon::prelude::*;
use std::path::Path;

/// Diff generated files against what's on disk.
///
/// For Rust files, both sides are formatted with rustfmt before comparison.
/// For all files, whitespace is normalized (trailing whitespace stripped,
/// trailing newline ensured) so that formatter-only diffs are ignored.
pub fn diff_files(files: &[(Language, Vec<GeneratedFile>)], base_dir: &Path) -> anyhow::Result<Vec<String>> {
    let all_items: Vec<_> = files
        .iter()
        .flat_map(|(lang, lang_files)| lang_files.iter().map(move |f| (*lang, f)))
        .collect();

    let diffs: Vec<String> = all_items
        .par_iter()
        .filter_map(|(lang, file)| {
            let full_path = base_dir.join(&file.path);
            // Binary output is compared as bytes or not at all. `read_to_string` fails on a
            // jar and `unwrap_or_default()` turns that failure into an empty string, which
            // then differs from the base64 `file.content` no matter what is on disk -- so
            // every repo with a kotlin-android target reported `gradle-wrapper.jar` as
            // pending on every run, forever, and `alef diff --exit-code` could never
            // return 0. A permanently un-actionable diff entry is worse than none: it
            // teaches the reader to skim past the list where real drift appears. ~keep
            if super::binary::is_base64_binary_output(&file.path) {
                let matches = super::binary::decode_base64_binary(&file.path, &file.content)
                    .ok()
                    .zip(std::fs::read(&full_path).ok())
                    .is_some_and(|(generated, on_disk)| generated == on_disk);
                return (!matches).then(|| format!("[{lang}] {}", file.path.display()));
            }
            let existing = std::fs::read_to_string(&full_path).unwrap_or_default();
            let is_rust = file.path.extension().is_some_and(|ext| ext == "rs");
            let generated_content = if file.generated_header
                && file.path.extension().is_some_and(|extension| extension == "toml")
                && !existing.is_empty()
            {
                super::scaffold::merge_managed_toml_preview(&existing, &file.content, base_dir, &file.path)
                    .unwrap_or_else(|_| file.content.clone())
            } else {
                file.content.clone()
            };
            let generated = normalize_content(base_dir, &file.path, &generated_content);
            let generated = if file.generated_header {
                super::write::ensure_generated_header(&file.path, &generated)
            } else {
                generated
            };
            let on_disk = if is_rust {
                format_rust_content(base_dir, &full_path, &existing)
            } else {
                existing
            };
            let on_disk_body = hash::strip_hash_line(&on_disk);
            if normalize_whitespace(&on_disk_body) != normalize_whitespace(&generated) {
                Some(format!("[{lang}] {}", file.path.display()))
            } else {
                None
            }
        })
        .collect();

    Ok(diffs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::backend::GeneratedFile;
    use base64::Engine;

    const WRAPPER_BYTES: &[u8] = &[0x50, 0x4b, 0x03, 0x04, 0x00, 0xff, 0xfe, 0x01];
    const JAR_PATH: &str = "packages/kotlin-android/gradle/wrapper/gradle-wrapper.jar";

    fn jar_output(content: String) -> Vec<(Language, Vec<GeneratedFile>)> {
        vec![(
            Language::KotlinAndroid,
            vec![GeneratedFile {
                path: std::path::PathBuf::from(JAR_PATH),
                content,
                generated_header: false,
            }],
        )]
    }

    fn seed_jar(base: &Path, bytes: &[u8]) {
        let full = base.join(JAR_PATH);
        std::fs::create_dir_all(full.parent().expect("parent")).expect("mkdir");
        std::fs::write(&full, bytes).expect("seed jar");
    }

    /// The permanent false positive. A jar whose bytes on disk are exactly what alef would
    /// write is not a pending change, and reporting it as one on every run in every repo is
    /// an entry nobody can ever act on -- which is how the entries that *do* matter, listed
    /// right beside it, stop being read. ~keep
    #[test]
    fn a_binary_output_matching_its_generated_bytes_reports_no_diff() {
        let dir = tempfile::tempdir().expect("tempdir");
        let encoded = base64::engine::general_purpose::STANDARD.encode(WRAPPER_BYTES);
        seed_jar(dir.path(), WRAPPER_BYTES);

        let diffs = diff_files(&jar_output(encoded), dir.path()).expect("diff");

        assert_eq!(diffs, Vec::<String>::new(), "an unchanged jar must not be reported");
    }

    /// The paired positive control: the fix must not be "never report binaries", which would
    /// pass the assertion above while examining nothing. ~keep
    #[test]
    fn a_binary_output_differing_from_its_generated_bytes_is_reported() {
        let dir = tempfile::tempdir().expect("tempdir");
        let encoded = base64::engine::general_purpose::STANDARD.encode(WRAPPER_BYTES);
        seed_jar(dir.path(), &[0x50, 0x4b, 0x03, 0x04]);

        let diffs = diff_files(&jar_output(encoded), dir.path()).expect("diff");

        assert_eq!(
            diffs,
            vec![format!("[kotlin_android] {JAR_PATH}")],
            "a jar whose bytes differ from alef's is a real pending change"
        );
    }
}
