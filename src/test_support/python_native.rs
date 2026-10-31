use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) fn configure_python(command: &mut Command) {
    let python = std::env::var_os("PYO3_PYTHON").unwrap_or_else(|| {
        super::tool_path_with_stable_path("python3")
            .or_else(|| super::tool_path_with_stable_path("python"))
            .expect("native PyO3 fixture requires a Python interpreter")
            .into_os_string()
    });
    command.env("PYO3_PYTHON", &python);
    if let Some(variable) = loader_path_variable(std::env::consts::OS) {
        let library = python_library_directory(&python);
        prepend_library_path(command, variable, &library);
    }
}

fn loader_path_variable(os: &str) -> Option<&'static str> {
    match os {
        "macos" => Some("DYLD_LIBRARY_PATH"),
        "linux" => Some("LD_LIBRARY_PATH"),
        _ => None,
    }
}

fn python_library_directory(python: &OsStr) -> PathBuf {
    let output = super::spawn_from_stable_dir(python)
        .args([
            "-c",
            concat!(
                "import json, sysconfig; print(json.dumps([",
                "sysconfig.get_config_var('LIBDIR') or '', ",
                "sysconfig.get_config_var('LDLIBRARY') or sysconfig.get_config_var('INSTSONAME') or ''",
                "]))"
            ),
        ])
        .output()
        .expect("read the selected Python's shared library metadata");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let [library, filename]: [String; 2] =
        serde_json::from_slice(&output.stdout).expect("Python shared library metadata must be two strings");
    validated_library_directory(Path::new(&library), &filename)
}

fn validated_library_directory(library: &Path, filename: &str) -> PathBuf {
    assert_ne!(
        library.as_os_str(),
        OsStr::new(""),
        "Python reported an empty library directory"
    );
    assert_ne!(filename, "", "Python reported an empty library filename");
    assert!(
        library.join(filename).is_file(),
        "the selected Python's library must exist: {}",
        library.join(filename).display()
    );
    library.to_path_buf()
}

fn prepend_library_path(command: &mut Command, variable: &str, library: &Path) {
    let inherited = std::env::var_os(variable);
    prepend_with_inherited(command, variable, library, inherited.as_deref());
}

fn prepend_with_inherited(command: &mut Command, variable: &str, library: &Path, inherited: Option<&OsStr>) {
    let inherited = match command.get_envs().find(|(key, _)| *key == variable) {
        Some((_, value)) => value,
        None => inherited,
    };
    let inherited = inherited.filter(|value| !value.is_empty());
    let paths = std::iter::once(library.to_path_buf()).chain(inherited.into_iter().flat_map(std::env::split_paths));
    command.env(
        variable,
        std::env::join_paths(paths).expect("Python library paths must form a valid loader path"),
    );
}

#[cfg(test)]
fn child_loader_paths(command: &Command, variable: &str) -> Vec<PathBuf> {
    let value = command
        .get_envs()
        .find(|(key, _)| *key == variable)
        .and_then(|(_, value)| value)
        .expect("child loader path");
    std::env::split_paths(value).collect()
}

#[test]
fn should_prepend_python_without_losing_explicit_child_loader_paths() {
    let python = Path::new("selected Python/lib");
    for variable in ["DYLD_LIBRARY_PATH", "LD_LIBRARY_PATH"] {
        let inherited = std::env::join_paths(["existing/lib", "another/lib"]).unwrap();
        let mut command = Command::new("fixture");
        command.env(variable, inherited);
        prepend_library_path(&mut command, variable, python);
        assert_eq!(
            child_loader_paths(&command, variable),
            [python.to_path_buf(), "existing/lib".into(), "another/lib".into()]
        );
    }
}

#[test]
fn should_set_python_library_after_the_child_removes_the_inherited_loader_path() {
    let python = Path::new("selected Python/lib");
    let inherited = std::env::join_paths(["parent/lib"]).unwrap();
    for variable in ["DYLD_LIBRARY_PATH", "LD_LIBRARY_PATH"] {
        let mut command = Command::new("fixture");
        command.env_remove(variable);
        prepend_with_inherited(&mut command, variable, python, Some(&inherited));
        assert_eq!(child_loader_paths(&command, variable), [python.to_path_buf()]);
    }
}

#[test]
fn should_preserve_inherited_python_loader_paths_without_mutating_the_process() {
    let python = Path::new("selected Python/lib");
    let inherited = std::env::join_paths(["parent/lib", "another parent/lib"]).unwrap();
    for variable in ["DYLD_LIBRARY_PATH", "LD_LIBRARY_PATH"] {
        let before = std::env::var_os(variable);
        let mut command = Command::new("fixture");
        prepend_with_inherited(&mut command, variable, python, Some(&inherited));
        assert_eq!(
            child_loader_paths(&command, variable),
            [python.to_path_buf(), "parent/lib".into(), "another parent/lib".into()]
        );
        assert_eq!(std::env::var_os(variable), before);
    }
}

#[test]
fn should_select_the_native_python_loader_variable_for_linux_and_macos() {
    assert_eq!(loader_path_variable("linux"), Some("LD_LIBRARY_PATH"));
    assert_eq!(loader_path_variable("macos"), Some("DYLD_LIBRARY_PATH"));
    assert_eq!(loader_path_variable("windows"), None);
}

#[test]
fn should_require_the_selected_python_metadata_to_name_an_existing_library() {
    let directory = tempfile::tempdir().unwrap();
    let filename = "selected-libpython.so";
    assert!(std::panic::catch_unwind(|| validated_library_directory(directory.path(), filename)).is_err());
    std::fs::write(directory.path().join(filename), b"library fixture").unwrap();
    assert_eq!(
        validated_library_directory(directory.path(), filename),
        directory.path()
    );
    for (library, name) in [(Path::new(""), filename), (directory.path(), "")] {
        assert!(std::panic::catch_unwind(|| validated_library_directory(library, name)).is_err());
    }
}
