use alef::core::backend::GeneratedFile;
use alef::core::config::{Language, NewAlefConfig};
use alef::core::ir::ApiSurface;
use std::path::Path;

const DIRECTORY: &str = ".linkedFramework(\"OpenDirectory\", .when(platforms: [.macOS])),";
const SYSTEM: &str = ".linkedFramework(\"SystemConfiguration\", .when(platforms: [.macOS])),";

fn manifests() -> Vec<GeneratedFile> {
    let raw: NewAlefConfig = toml::from_str(
        r#"[workspace]
languages = ["swift"]
[[crates]]
name = "my-lib"
sources = []
[crates.package_metadata]
repository = "https://github.com/example/my-lib"
"#,
    )
    .expect("fixture config parses");
    let config = raw.resolve().expect("fixture resolves").remove(0);
    let files = alef::scaffold::scaffold(&ApiSurface::default(), &config, &[Language::Swift]).expect("real scaffold");
    let manifests: Vec<_> = files
        .into_iter()
        .filter(|file| {
            file.path == Path::new("Package.swift") || file.path == Path::new("packages/swift/Package.swift")
        })
        .collect();
    assert_eq!(manifests.len(), 2, "both real dev and distribution outputs must exist");
    assert!(manifests.iter().all(|file| !file.generated_header));
    manifests
}

fn manifest(relative: &str) -> GeneratedFile {
    manifests()
        .into_iter()
        .find(|file| file.path == Path::new(relative))
        .expect("requested real manifest")
}

fn code(content: &str) -> String {
    content
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn bridge(content: &str) -> &str {
    let start = content.find("name: \"RustBridge\",").expect("real RustBridge target");
    let end = start + content[start..].find("\n    ),").expect("RustBridge closing target");
    let target = &content[start..end];
    assert!(
        target.contains("Sources/RustBridge\""),
        "actual bridge source target required"
    );
    assert!(target.contains("linkerSettings: ["), "actual linker settings required");
    target
}

fn without_directory(content: &str) -> String {
    content
        .lines()
        .filter(|line| line.trim() != DIRECTORY)
        .map(|line| format!("{line}\n"))
        .collect()
}

fn add_after_anchor(content: &str, added: &str) -> String {
    assert_eq!(
        content.matches(SYSTEM).count(),
        1,
        "one recognized system framework anchor"
    );
    content.replace(SYSTEM, &format!("{SYSTEM}\n        {added}"))
}

fn run_writer(relative: &str, existing: &str) -> String {
    let root = tempfile::tempdir().expect("private fixture");
    let path = root.path().join(relative);
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(&path, existing).expect("seed existing create-only manifest");
    let mut file = manifest("packages/swift/Package.swift");
    file.path = relative.into();
    alef::cli::pipeline::write_scaffold_files_report(&[file], root.path(), false).expect("canonical write path");
    std::fs::read_to_string(path).expect("read actual artifact")
}

fn assert_framework_and_preservation(after: &str, before: &str) {
    let target = code(bridge(after));
    assert_eq!(
        target.matches(DIRECTORY).count(),
        1,
        "bridge needs exactly one macOS-only OpenDirectory link"
    );
    assert_eq!(
        without_directory(after),
        before,
        "only the missing framework line may change"
    );
}

fn existing_manifest_migrates_once(relative: &str) {
    let file = manifest(relative);
    let legacy = without_directory(&file.content);
    assert!(code(bridge(&legacy)).contains(SYSTEM));
    assert!(!code(bridge(&legacy)).contains("OpenDirectory"));
    let root = tempfile::tempdir().expect("private fixture");
    let path = root.path().join(relative);
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(&path, &legacy).expect("legacy seed");
    alef::cli::pipeline::write_scaffold_files_report(std::slice::from_ref(&file), root.path(), false)
        .expect("first canonical write");
    let first = std::fs::read_to_string(&path).expect("first artifact");
    assert_framework_and_preservation(&first, &legacy);
    alef::cli::pipeline::write_scaffold_files_report(&[file], root.path(), false).expect("second canonical write");
    assert_eq!(
        std::fs::read_to_string(path).expect("second artifact"),
        first,
        "migration must reach a fixed point"
    );
}

#[test]
fn fresh_dev_scaffold_links_open_directory_only_for_macos() {
    let file = manifest("packages/swift/Package.swift");
    let target = code(bridge(&file.content));
    assert!(target.contains(SYSTEM));
    assert_eq!(
        target.matches(DIRECTORY).count(),
        1,
        "actual dev scaffold lacks the native framework"
    );
}

#[test]
fn fresh_distribution_scaffold_links_open_directory_only_for_macos() {
    let file = manifest("Package.swift");
    let target = code(bridge(&file.content));
    assert!(target.contains(SYSTEM));
    assert_eq!(
        target.matches(DIRECTORY).count(),
        1,
        "actual distribution scaffold lacks the native framework"
    );
}

#[test]
fn existing_dev_manifest_is_repaired_through_canonical_writer_once() {
    existing_manifest_migrates_once("packages/swift/Package.swift");
}

#[test]
fn existing_distribution_manifest_is_repaired_through_canonical_writer_once() {
    existing_manifest_migrates_once("Package.swift");
}

#[test]
fn migration_preserves_custom_frameworks_flags_and_other_manifest_content() {
    let legacy = without_directory(&manifest("packages/swift/Package.swift").content);
    let custom = add_after_anchor(
        &legacy,
        ".linkedFramework(\"AppKit\", .when(platforms: [.macOS])),\n        .unsafeFlags([\"-L/consumer-owned\"]),",
    );
    let after = run_writer("packages/swift/Package.swift", &custom);
    assert_framework_and_preservation(&after, &custom);
}

#[test]
fn another_target_framework_does_not_satisfy_rust_bridge_requirement() {
    let legacy = without_directory(&manifest("packages/swift/Package.swift").content);
    let end = legacy.rfind("  ]").expect("actual targets end");
    let other =
        format!("    .target(name: \"OtherTarget\", path: \"Sources/Other\", linkerSettings: [{DIRECTORY}]),\n");
    let custom = format!("{}{}{}", &legacy[..end], other, &legacy[end..]);
    assert_eq!(custom.matches(DIRECTORY).count(), 1);
    assert!(!code(bridge(&custom)).contains("OpenDirectory"));
    let after = run_writer("packages/swift/Package.swift", &custom);
    assert_framework_and_preservation(&after, &custom);
    assert_eq!(after.matches(DIRECTORY).count(), 2, "other target must be preserved");
}

#[test]
fn already_custom_open_directory_platform_conditions_are_not_rewritten() {
    let legacy = without_directory(&manifest("packages/swift/Package.swift").content);
    let custom = add_after_anchor(
        &legacy,
        ".linkedFramework(\"OpenDirectory\", .when(platforms: [.macOS, .iOS])),",
    );
    assert_eq!(run_writer("packages/swift/Package.swift", &custom), custom);
}

#[test]
fn unrecognized_linker_anchor_is_left_byte_identical() {
    let legacy = without_directory(&manifest("packages/swift/Package.swift").content);
    let custom = legacy.replace(
        SYSTEM,
        ".linkedFramework(\"ConsumerFramework\", .when(platforms: [.macOS])),",
    );
    assert!(!custom.contains(SYSTEM));
    assert_eq!(run_writer("packages/swift/Package.swift", &custom), custom);
}

#[test]
fn ambiguous_duplicate_rust_bridge_targets_are_not_patched() {
    let legacy = without_directory(&manifest("packages/swift/Package.swift").content);
    let end = legacy.rfind("  ]").expect("actual targets end");
    let extra = format!("    .target(\n      {}\n    ),\n", bridge(&legacy));
    let custom = format!("{}{}{}", &legacy[..end], extra, &legacy[end..]);
    assert_eq!(custom.matches("name: \"RustBridge\",").count(), 2);
    assert_eq!(run_writer("packages/swift/Package.swift", &custom), custom);
}

#[test]
fn unrelated_manifest_path_is_not_migrated() {
    let legacy = without_directory(&manifest("packages/swift/Package.swift").content);
    assert_eq!(run_writer("custom/Package.swift", &legacy), legacy);
}

#[test]
fn canonical_writer_rejects_escape_without_modifying_outside_file() {
    let parent = tempfile::tempdir().expect("private fixture");
    let root = parent.path().join("project");
    std::fs::create_dir(&root).expect("project");
    let outside = parent.path().join("Package.swift");
    let original = manifest("packages/swift/Package.swift").content;
    std::fs::write(&outside, &original).expect("outside fixture");
    let mut file = manifest("packages/swift/Package.swift");
    file.path = "../Package.swift".into();
    assert!(alef::cli::pipeline::write_scaffold_files_report(&[file], &root, false).is_err());
    assert_eq!(std::fs::read_to_string(outside).expect("outside artifact"), original);
}

#[test]
fn commented_framework_does_not_satisfy_bridge_link() {
    let legacy = without_directory(&manifest("packages/swift/Package.swift").content);
    let custom = add_after_anchor(&legacy, &format!("// {DIRECTORY}"));
    assert_framework_and_preservation(&run_writer("packages/swift/Package.swift", &custom), &custom);
}

#[test]
fn unsafe_flags_string_does_not_satisfy_bridge_link() {
    let legacy = without_directory(&manifest("packages/swift/Package.swift").content);
    let custom = add_after_anchor(&legacy, r#".unsafeFlags([".linkedFramework(\"OpenDirectory\")"]),"#);
    assert_framework_and_preservation(&run_writer("packages/swift/Package.swift", &custom), &custom);
}

#[test]
fn commented_system_anchor_is_not_a_framework_call() {
    let legacy = without_directory(&manifest("packages/swift/Package.swift").content);
    let custom = legacy.replace(SYSTEM, &format!("// {SYSTEM}"));
    assert_eq!(run_writer("packages/swift/Package.swift", &custom), custom);
}

#[test]
fn commented_target_does_not_make_real_bridge_ambiguous() {
    let legacy = without_directory(&manifest("packages/swift/Package.swift").content);
    let custom = format!("/* .target(name: \"RustBridge\", path: \"fake\"), */\n{legacy}");
    let after = run_writer("packages/swift/Package.swift", &custom);
    assert_eq!(after.matches(DIRECTORY).count(), 1);
    assert_eq!(without_directory(&after), custom);
}

#[test]
fn inline_closed_linker_list_is_not_patched_outside_the_list() {
    let legacy = without_directory(&manifest("packages/swift/Package.swift").content);
    let custom = legacy.replace(&format!("{SYSTEM}\n      ]"), &format!("{SYSTEM} ]"));
    assert_ne!(custom, legacy);
    assert_eq!(run_writer("packages/swift/Package.swift", &custom), custom);
}

#[test]
fn trailing_multiline_comment_keeps_framework_executable_and_migration_idempotent() {
    let relative = "packages/swift/Package.swift";
    let file = manifest(relative);
    let legacy = without_directory(&file.content);
    let custom = legacy.replace(SYSTEM, &format!("{SYSTEM} /* consumer comment\ncontinued\n*/"));
    assert_ne!(custom, legacy);
    let root = tempfile::tempdir().expect("private fixture");
    let path = root.path().join(relative);
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(&path, &custom).expect("existing manifest with trailing block comment");
    alef::cli::pipeline::write_scaffold_files_report(std::slice::from_ref(&file), root.path(), false)
        .expect("first canonical migration");
    let first = std::fs::read_to_string(&path).expect("first artifact");
    alef::cli::pipeline::write_scaffold_files_report(&[file], root.path(), false).expect("second canonical migration");
    let second = std::fs::read_to_string(&path).expect("second artifact");
    assert_eq!(
        second, first,
        "framework migration must not add a commented-out call on each run"
    );
    assert_eq!(first.matches(DIRECTORY).count(), 1);
    let call = first.find(DIRECTORY).expect("framework call");
    let comment = first.find("/* consumer comment").expect("preserved comment");
    let end = comment + first[comment..].find("*/").expect("comment end") + 2;
    assert!(
        call < comment || call >= end,
        "framework call must be outside the block comment"
    );
    assert_eq!(without_directory(&first), custom, "consumer bytes must be preserved");
}

#[cfg(unix)]
fn existing_manifest_preserves_permissions(relative: &str, mode: u32) {
    use std::os::unix::fs::PermissionsExt;

    let file = manifest(relative);
    let legacy = without_directory(&file.content);
    let root = tempfile::tempdir().expect("private permission fixture");
    let path = root.path().join(relative);
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(&path, &legacy).expect("legacy seed");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).expect("set explicit fixture mode");
    assert_eq!(
        std::fs::metadata(&path).expect("seed metadata").permissions().mode() & 0o7777,
        mode,
        "fixture must begin with the requested permissions"
    );
    alef::cli::pipeline::write_scaffold_files_report(std::slice::from_ref(&file), root.path(), false)
        .expect("first canonical migration");
    let first = std::fs::read_to_string(&path).expect("first artifact");
    assert_framework_and_preservation(&first, &legacy);
    assert_eq!(
        std::fs::metadata(&path).expect("first metadata").permissions().mode() & 0o7777,
        mode,
        "migration must preserve manifest permissions for {relative}"
    );
    alef::cli::pipeline::write_scaffold_files_report(&[file], root.path(), false).expect("second canonical write");
    assert_eq!(std::fs::read_to_string(&path).expect("second artifact"), first);
    assert_eq!(
        std::fs::metadata(&path).expect("second metadata").permissions().mode() & 0o7777,
        mode,
        "idempotent regeneration must preserve manifest permissions for {relative}"
    );
}

#[cfg(unix)]
#[test]
fn existing_dev_manifest_preserves_permissions_and_reaches_fixed_point() {
    for mode in [0o640, 0o644] {
        existing_manifest_preserves_permissions("packages/swift/Package.swift", mode);
    }
}

#[cfg(unix)]
#[test]
fn existing_distribution_manifest_preserves_permissions_and_reaches_fixed_point() {
    for mode in [0o640, 0o644] {
        existing_manifest_preserves_permissions("Package.swift", mode);
    }
}
