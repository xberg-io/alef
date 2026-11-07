use super::*;
use std::path::Path;

fn input<'a>() -> ExecutableFingerprintInput<'a> {
    ExecutableFingerprintInput {
        crate_name: "demo",
        component: "sh",
        target: "x86_64-unknown-linux-gnu",
        release: true,
        file: "sh",
        build: "go build -o {out} .",
        contracts: &[],
        distribution: None,
        toolchain: "go version go1.26.0 linux/amd64",
        env: &[],
        extras: &[],
    }
}

fn files() -> Vec<(String, String)> {
    vec![
        ("sh/main.go".into(), "a".repeat(64)),
        ("sh/go.mod".into(), "b".repeat(64)),
    ]
}

#[test]
fn fingerprint_is_stable_and_order_independent() {
    let first = compute_executable(&input(), &files());
    let mut reversed = files();
    reversed.reverse();
    assert_eq!(first, compute_executable(&input(), &reversed));
    assert_eq!(first.len(), 64);
}

#[test]
fn every_declared_input_changes_the_fingerprint() {
    let base = compute_executable(&input(), &files());
    let mut changed = Vec::new();
    let mut other = input();
    other.target = "aarch64-apple-darwin";
    changed.push(("target", compute_executable(&other, &files())));
    let mut other = input();
    other.release = false;
    changed.push(("profile", compute_executable(&other, &files())));
    let mut other = input();
    other.file = "sh.exe";
    changed.push(("file", compute_executable(&other, &files())));
    let mut other = input();
    other.build = "go build -trimpath -o {out} .";
    changed.push(("build command", compute_executable(&other, &files())));
    let mut other = input();
    other.toolchain = "go version go1.26.1 linux/amd64";
    changed.push(("toolchain", compute_executable(&other, &files())));
    let mut other = input();
    other.toolchain = "";
    changed.push(("no toolchain", compute_executable(&other, &files())));
    let env = [("GOFLAGS".to_string(), "-mod=vendor".to_string())];
    let mut other = input();
    other.env = &env;
    changed.push(("env", compute_executable(&other, &files())));
    let extras = [("image".to_string(), "x".to_string())];
    let mut other = input();
    other.extras = &extras;
    changed.push(("extra", compute_executable(&other, &files())));
    let mut edited = files();
    edited[0].1 = "c".repeat(64);
    changed.push(("file content", compute_executable(&input(), &edited)));
    let mut added = files();
    added.push(("sh/extra.go".into(), "d".repeat(64)));
    changed.push(("added file", compute_executable(&input(), &added)));
    for (what, hash) in &changed {
        assert_ne!(*hash, base, "{what} must change the fingerprint");
    }
    let distinct = changed
        .iter()
        .map(|(_, hash)| hash)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(distinct.len(), changed.len(), "inputs must not collide with each other");
}

#[test]
fn the_alef_version_does_not_enter_the_fingerprint() {
    // The struct has no alef-version field: upgrading alef alone must keep reusing a build.
    let hash = compute_executable(&input(), &files());
    assert_eq!(hash, compute_executable(&input(), &files()));
}

#[test]
fn portable_hash_covers_files_and_build_only() {
    let base = portable_executable_hash(&files(), "make {out}");
    assert_eq!(base, portable_executable_hash(&files(), "make {out}"));
    assert_ne!(base, portable_executable_hash(&files(), "make -j {out}"));
    let mut edited = files();
    edited[1].1 = "e".repeat(64);
    assert_ne!(base, portable_executable_hash(&edited, "make {out}"));
}

#[test]
fn inputs_expand_directories_and_globs_and_reject_empty_matches() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_str().unwrap().replace('\\', "/");
    std::fs::create_dir_all(dir.path().join("src/nested")).unwrap();
    std::fs::create_dir_all(dir.path().join("src/.git")).unwrap();
    std::fs::write(dir.path().join("src/a.go"), "a").unwrap();
    std::fs::write(dir.path().join("src/nested/b.go"), "b").unwrap();
    std::fs::write(dir.path().join("src/.git/HEAD"), "ignored").unwrap();
    std::fs::write(dir.path().join("go.mod"), "module x").unwrap();
    std::fs::write(dir.path().join("README.md"), "docs").unwrap();

    let by_dir = executable_inputs(&[format!("{root}/src")], Path::new(""), &[]).unwrap();
    assert_eq!(by_dir.len(), 2, "{by_dir:?}");
    assert!(by_dir.iter().all(|(label, _)| !label.contains(".git")), "{by_dir:?}");

    let mixed = executable_inputs(&[format!("{root}/src"), format!("{root}/go.*")], Path::new(""), &[]).unwrap();
    assert_eq!(mixed.len(), 3, "{mixed:?}");
    assert!(mixed.iter().any(|(label, _)| label.ends_with("go.mod")));
    assert!(!mixed.iter().any(|(label, _)| label.ends_with("README.md")));

    let before = executable_inputs(&[format!("{root}/src")], Path::new(""), &[]).unwrap();
    std::fs::write(dir.path().join("src/a.go"), "changed").unwrap();
    assert_ne!(
        before,
        executable_inputs(&[format!("{root}/src")], Path::new(""), &[]).unwrap()
    );

    let error = executable_inputs(&[format!("{root}/nothing/**/*.go")], Path::new(""), &[])
        .unwrap_err()
        .to_string();
    assert!(error.contains("matched no files"), "{error}");
}

#[cfg(unix)]
#[test]
fn toolchain_identity_runs_the_command_and_rejects_failure_and_silence() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(toolchain_identity("echo  tool 1.2  ", dir.path()).unwrap(), "tool 1.2");
    assert!(toolchain_identity("exit 3", dir.path()).is_err());
    let silent = toolchain_identity("true", dir.path()).unwrap_err().to_string();
    assert!(silent.contains("printed nothing"), "{silent}");
}

fn labels(inputs: &[(String, String)]) -> Vec<&str> {
    inputs.iter().map(|(label, _)| label.as_str()).collect()
}

#[test]
fn record_keys_are_length_prefixed_so_a_key_cannot_forge_a_record() {
    let forged = [("a=1:b\nextra:c".to_string(), "d".to_string())];
    let genuine = [("a".to_string(), "b".to_string()), ("c".to_string(), "d".to_string())];
    let mut one = input();
    one.extras = &forged;
    let mut two = input();
    two.extras = &genuine;
    assert_ne!(compute_executable(&one, &files()), compute_executable(&two, &files()));
    assert_eq!(EXECUTABLE_FINGERPRINT_VERSION, 2);
}

#[cfg(unix)]
#[test]
fn symlinks_are_hashed_by_target_and_never_followed() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(src.join("sub")).unwrap();
    std::fs::write(src.join("a.go"), "a").unwrap();
    std::fs::write(dir.path().join("real.go"), "r").unwrap();
    // A cycle back to the parent, a dangling link and a link to a file outside the tree.
    symlink("..", src.join("sub/up")).unwrap();
    symlink("missing.go", src.join("dangling")).unwrap();
    symlink("../real.go", src.join("outside.go")).unwrap();
    let patterns = [src.to_str().unwrap().to_string()];

    let before = executable_inputs(&patterns, Path::new(""), &[]).unwrap();
    assert_eq!(before.len(), 4, "{before:?}");

    std::fs::remove_file(src.join("dangling")).unwrap();
    symlink("other.go", src.join("dangling")).unwrap();
    let retargeted = executable_inputs(&patterns, Path::new(""), &[]).unwrap();
    assert_ne!(before, retargeted, "retargeting a link must change the hash");

    // The content behind a link is not an input: only its target string is.
    std::fs::write(dir.path().join("real.go"), "changed").unwrap();
    assert_eq!(retargeted, executable_inputs(&patterns, Path::new(""), &[]).unwrap());
}

#[cfg(unix)]
#[test]
fn a_symlinked_producer_directory_is_read_but_links_below_it_are_not() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("real")).unwrap();
    std::fs::write(dir.path().join("real/a.go"), "a").unwrap();
    symlink("..", dir.path().join("real/loop")).unwrap();
    symlink("real", dir.path().join("link")).unwrap();
    let inputs = executable_inputs(
        &[dir.path().join("link").to_str().unwrap().to_string()],
        Path::new(""),
        &[],
    )
    .unwrap();
    assert_eq!(inputs.len(), 2, "{inputs:?}");
}

#[test]
fn labels_are_relative_to_the_config_directory_not_the_working_directory() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("exe")).unwrap();
    std::fs::write(dir.path().join("exe/main.go"), "m").unwrap();
    std::fs::write(dir.path().join("go.work"), "w").unwrap();
    let root = dir.path().to_str().unwrap().replace('\\', "/");
    let from_sub = executable_inputs(
        &[format!("{root}/exe"), format!("{root}/go.work")],
        Path::new(&root),
        &[],
    )
    .unwrap();
    assert_eq!(labels(&from_sub), ["exe/main.go", "go.work"]);
}

#[test]
fn git_is_skipped_for_glob_matches_and_excluded_directories_are_not_inputs() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_str().unwrap().replace('\\', "/");
    std::fs::create_dir_all(dir.path().join("p/.git")).unwrap();
    std::fs::create_dir_all(dir.path().join("p/.alef/build")).unwrap();
    std::fs::write(dir.path().join("p/.git/HEAD"), "h").unwrap();
    std::fs::write(dir.path().join("p/.alef/build/out"), "o").unwrap();
    std::fs::write(dir.path().join("p/main.go"), "m").unwrap();
    let globbed = executable_inputs(&[format!("{root}/p/*")], Path::new(&root), &[]).unwrap();
    assert_eq!(labels(&globbed), ["p/.alef/build/out", "p/main.go"]);
    let exclude = [dir.path().join("p/.alef")];
    let inputs = executable_inputs(&[format!("{root}/p/*")], Path::new(&root), &exclude).unwrap();
    assert_eq!(labels(&inputs), ["p/main.go"]);
    let literal = executable_inputs(&[format!("{root}/p")], Path::new(&root), &exclude).unwrap();
    assert_eq!(labels(&literal), ["p/main.go"]);
}

#[test]
fn a_missing_shell_is_reported_with_the_fix() {
    let empty = tempfile::tempdir().unwrap();
    let error = ensure_shell(Some(empty.path().as_os_str())).unwrap_err().to_string();
    assert!(error.contains("no `sh` was found on PATH"), "{error}");
    assert!(ensure_shell(None).is_err());
    let with_sh = tempfile::tempdir().unwrap();
    let name = if cfg!(windows) { "sh.exe" } else { "sh" };
    std::fs::write(with_sh.path().join(name), "").unwrap();
    assert!(ensure_shell(Some(with_sh.path().as_os_str())).is_ok());
}
