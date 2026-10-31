//! Skill descriptions are listed in every consumer's start context, and the org's
//! context-exposure gate (`--fail-on-long-descriptions`) rejects any over 250 characters. The
//! alef skill ships into consumer repos, so an over-long description fails their CI, not ours.

use std::path::{Path, PathBuf};

const MAX_DESCRIPTION_CHARS: usize = 250;

fn skill_files(root: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(root)
        .unwrap_or_else(|error| panic!("read {}: {error}", root.display()))
        .map(|entry| entry.expect("skill dir entry").path().join("SKILL.md"))
        .filter(|path| path.is_file())
        .collect();
    files.sort();
    files
}

fn description(skill: &Path) -> String {
    let text = std::fs::read_to_string(skill).expect("read SKILL.md");
    let frontmatter = text
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---").map(|(front, _)| front))
        .unwrap_or_else(|| panic!("{} has no frontmatter", skill.display()));
    let parsed: serde_json::Value = serde_saphyr::from_str(frontmatter).expect("frontmatter YAML");
    parsed["description"]
        .as_str()
        .unwrap_or_else(|| panic!("{} has no description", skill.display()))
        .to_string()
}

#[test]
fn every_skill_description_fits_the_context_exposure_limit() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let skills: Vec<PathBuf> = ["skills", ".ai-rulez/skills"]
        .iter()
        .flat_map(|dir| skill_files(&root.join(dir)))
        .collect();
    assert!(
        skills.iter().any(|path| path.starts_with(root.join("skills/alef"))),
        "the shipped alef skill must be among the checked files: {skills:?}"
    );

    let too_long: Vec<String> = skills
        .iter()
        .map(|path| (path, description(path).chars().count()))
        .filter(|(_, length)| *length > MAX_DESCRIPTION_CHARS)
        .map(|(path, length)| format!("{}: {length}", path.display()))
        .collect();
    assert!(
        too_long.is_empty(),
        "skill descriptions over {MAX_DESCRIPTION_CHARS} characters: {too_long:?}"
    );
}
