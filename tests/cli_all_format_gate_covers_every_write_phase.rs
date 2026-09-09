/// Regression for alef #119: `alef all` used to gate its whole-tree formatting pass on
/// `changed_languages`, a set populated only when the bindings, service-API, or stub write
/// phase reported a byte-level change. Every other write phase -- scaffold, public API, e2e,
/// README, docs -- could write new content on disk without ever being consulted by that gate.
/// A steady-state run where only README output changed (e.g. because bindings were entirely
/// skipped by the per-language lang-hash cache while README content still differed) left
/// `changed_languages` empty, so the whole-tree `poly fmt` convergence pass never ran, and any
/// stray unformatted file elsewhere in the tree stayed unformatted forever -- with no second
/// `alef all` run ever fixing it, since the same narrow gate applied every time.
///
/// This is reproduced here with a single `python` target (no FFI build, no post-build step,
/// so the run is fast and needs no toolchain beyond alef's own generation pipeline):
/// 1. `alef all` once, from scratch -- a full write that also runs formatting.
/// 2. Delete the generated `README.md` and drop a deliberately unformatted, alef-unmanaged
///    `messy.py` file inside the python package directory.
/// 3. `alef all` again -- bindings/stubs/service-API are byte-identical to what's on disk (the
///    per-language cache hits), so only the README write phase reports a change. Under the old
///    gate this second run's log has no "Formatting generated files..." line at all, and
///    `messy.py` is left exactly as written (`x=1`).
/// 4. A third `alef all` run must make no further changes -- the tree is formatted and hash-
///    stable, not oscillating.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

fn alef_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_alef"))
}

fn write_fixture(root: &Path) {
    fs::create_dir_all(root.join("src")).expect("create fixture source directory");
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"format-gate-fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .expect("write fixture Cargo.toml");
    fs::write(
        root.join("src/lib.rs"),
        "pub struct Record { pub value: String }\n\npub fn record_value(record: Record) -> String { record.value }\n",
    )
    .expect("write fixture source");
    fs::write(
        root.join("alef.toml"),
        format!(
            "[workspace]\nalef_version = \"{}\"\nlanguages = [\"python\"]\n\n\
             [[crates]]\nname = \"format-gate-fixture\"\nsources = [\"src/lib.rs\"]\n\
             version_from = \"Cargo.toml\"\n\n[crates.generate]\npublic_api = false\n",
            env!("CARGO_PKG_VERSION")
        ),
    )
    .expect("write alef config");
}

fn install_test_formatter(root: &Path) -> PathBuf {
    let bin_dir = root.join("test-bin");
    fs::create_dir(&bin_dir).expect("create test formatter directory");

    #[cfg(unix)]
    {
        let formatter = bin_dir.join("poly");
        fs::write(
            &formatter,
            "#!/bin/sh\nif [ -f packages/python/messy.py ]; then\n  printf 'x = 1\\n' > packages/python/messy.py\nfi\n",
        )
        .expect("write test formatter");
        let mut permissions = fs::metadata(&formatter)
            .expect("read test formatter metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&formatter, permissions).expect("make test formatter executable");
    }

    #[cfg(windows)]
    fs::write(
        bin_dir.join("poly.cmd"),
        "@echo off\r\nif exist packages\\python\\messy.py (\r\n  > packages\\python\\messy.py echo x = 1\r\n)\r\n",
    )
    .expect("write test formatter");

    bin_dir
}

fn run_all(root: &Path, formatter_bin: &Path) -> Output {
    let path = std::env::join_paths(
        std::iter::once(formatter_bin.to_path_buf())
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())),
    )
    .expect("construct PATH with test formatter");
    Command::new(alef_binary())
        .current_dir(root)
        .env("PATH", path)
        .arg("all")
        .output()
        .expect("run alef all")
}

#[test]
fn a_readme_only_change_still_formats_stray_files_and_stays_hash_stable() {
    let fixture = tempfile::tempdir().expect("create fixture directory");
    let root = fixture.path();
    write_fixture(root);
    let formatter_bin = install_test_formatter(root);

    let first = run_all(root, &formatter_bin);
    assert!(
        first.status.success(),
        "first `alef all` run must succeed: {}",
        String::from_utf8_lossy(&first.stderr)
    );

    let readme = root.join("packages/python/README.md");
    assert!(readme.is_file(), "first run must generate a README");
    fs::remove_file(&readme).expect("delete README to force a README-only change on the next run");

    let messy = root.join("packages/python/messy.py");
    fs::write(&messy, "x=1\n").expect("seed an unformatted, alef-unmanaged file");

    let second = run_all(root, &formatter_bin);
    assert!(
        second.status.success(),
        "second `alef all` run must succeed: {}",
        String::from_utf8_lossy(&second.stderr)
    );
    let second_stdout_and_stderr = format!(
        "{}{}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr)
    );
    assert!(
        readme.is_file(),
        "the deleted README must be regenerated on the second run"
    );

    assert_eq!(
        fs::read_to_string(&messy).expect("read messy.py after the second run"),
        if cfg!(windows) { "x = 1\r\n" } else { "x = 1\n" },
        "a run whose only reported change was the README must still run the whole-tree \
         formatting pass and reformat every other file under the tree, including this \
         alef-unmanaged stray file -- if this still reads `x=1`, the format gate is still \
         narrowly keyed to bindings/service-API/stubs only. Full run output:\n{second_stdout_and_stderr}"
    );

    let messy_after_second = fs::read_to_string(&messy).expect("read messy.py again");
    let third = run_all(root, &formatter_bin);
    assert!(
        third.status.success(),
        "third `alef all` run must succeed: {}",
        String::from_utf8_lossy(&third.stderr)
    );
    assert_eq!(
        fs::read_to_string(&messy).expect("read messy.py after the third run"),
        messy_after_second,
        "a converged tree must be hash-stable: a third run must not change what the second \
         run's formatting pass already settled"
    );
}
