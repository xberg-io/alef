use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

fn dependency_dirs() -> &'static [PathBuf] {
    static DIRECTORIES: OnceLock<Vec<PathBuf>> = OnceLock::new();
    DIRECTORIES.get_or_init(|| {
        let executable = std::env::current_exe().expect("test executable path");
        let parent = executable.parent().expect("test executable directory");
        let Some(build) = parent
            .ancestors()
            .nth(3)
            .filter(|path| path.file_name().is_some_and(|name| name == "build"))
            .filter(|_| parent.file_name().is_some_and(|name| name == "out"))
        else {
            return vec![parent.to_path_buf()];
        };
        fs::read_dir(build)
            .expect("Cargo build directory")
            .filter_map(Result::ok)
            .filter_map(|entry| fs::read_dir(entry.path()).ok())
            .flatten()
            .filter_map(Result::ok)
            .map(|entry| entry.path().join("out"))
            .filter(|path| path.is_dir())
            .collect()
    })
}

pub(crate) fn dependency_rlibs(name: &str) -> Vec<PathBuf> {
    let prefix = format!("lib{name}-");
    let mut matches: Vec<_> = dependency_dirs()
        .iter()
        .filter_map(|directory| fs::read_dir(directory).ok())
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|file| file.starts_with(&prefix) && file.ends_with(".rlib"))
        })
        .filter_map(|entry| Some((entry.metadata().ok()?.modified().ok()?, entry.path())))
        .collect();
    matches.sort_unstable_by(|left, right| right.cmp(left));
    matches.into_iter().map(|(_, path)| path).collect()
}

pub(crate) fn add_dependency_search_paths(command: &mut Command) -> &mut Command {
    for directory in dependency_dirs() {
        command.arg("-L").arg(format!("dependency={}", directory.display()));
    }
    command
}
