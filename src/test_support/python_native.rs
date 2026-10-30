use std::path::Path;
use std::process::Command;

pub(crate) fn configure_python(command: &mut Command) {
    let python = std::env::var_os("PYO3_PYTHON").unwrap_or_else(|| {
        super::tool_path_with_stable_path("python3")
            .or_else(|| super::tool_path_with_stable_path("python"))
            .expect("native PyO3 fixture requires a Python interpreter")
            .into_os_string()
    });
    command.env("PYO3_PYTHON", &python);
    if cfg!(target_os = "macos") {
        let output = super::spawn_from_stable_dir(&python)
            .args([
                "-c",
                "import sysconfig; print(sysconfig.get_config_var('LIBDIR') or '')",
            ])
            .output()
            .expect("read the selected Python's library directory");
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let library = String::from_utf8(output.stdout).expect("Python library directory must be UTF-8");
        let library = library.trim_end_matches(['\r', '\n']);
        assert!(!library.is_empty(), "Python reported an empty shared library directory");
        // ~keep Mise's shared libpython uses @rpath on macOS; native fixture binaries have no Python rpath.
        prepend_library_path(command, Path::new(library));
    }
}

fn prepend_library_path(command: &mut Command, library: &Path) {
    let inherited = match command.get_envs().find(|(key, _)| *key == "DYLD_LIBRARY_PATH") {
        Some((_, value)) => value.map(std::ffi::OsStr::to_os_string),
        None => std::env::var_os("DYLD_LIBRARY_PATH"),
    };
    let inherited = inherited.filter(|value| !value.is_empty());
    let paths =
        std::iter::once(library.to_path_buf()).chain(inherited.as_deref().into_iter().flat_map(std::env::split_paths));
    command.env(
        "DYLD_LIBRARY_PATH",
        std::env::join_paths(paths).expect("Python library paths must form a valid loader path"),
    );
}

#[test]
fn should_prepend_python_without_losing_explicit_child_loader_paths() {
    let python = Path::new("selected Python/lib");
    let inherited = std::env::join_paths(["existing/lib", "another/lib"]).unwrap();
    let mut command = Command::new("fixture");
    command.env("DYLD_LIBRARY_PATH", inherited);
    prepend_library_path(&mut command, python);
    let value = command
        .get_envs()
        .find(|(key, _)| *key == "DYLD_LIBRARY_PATH")
        .and_then(|(_, value)| value)
        .unwrap();
    assert_eq!(
        std::env::split_paths(value).collect::<Vec<_>>(),
        [python.to_path_buf(), "existing/lib".into(), "another/lib".into()]
    );
}

#[test]
fn should_set_python_library_after_the_child_removes_the_inherited_loader_path() {
    let python = Path::new("selected Python/lib");
    let mut command = Command::new("fixture");
    command.env_remove("DYLD_LIBRARY_PATH");
    prepend_library_path(&mut command, python);
    let value = command
        .get_envs()
        .find(|(key, _)| *key == "DYLD_LIBRARY_PATH")
        .and_then(|(_, value)| value)
        .unwrap();
    assert_eq!(std::env::split_paths(value).collect::<Vec<_>>(), [python.to_path_buf()]);
}
