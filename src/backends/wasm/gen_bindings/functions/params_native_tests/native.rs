use std::path::{Path, PathBuf};
use std::process::{Command, Output};

pub(super) fn build(root: &Path) -> Output {
    let channel = committed_channel();
    let cargo = installed_tool(&channel, "cargo", root);
    let rustc = installed_tool(&channel, "rustc", root);
    let path = std::env::join_paths(
        std::iter::once(cargo.parent().unwrap().to_path_buf())
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())),
    )
    .unwrap();
    std::fs::write(root.join("mise.toml"), "[settings]\ntask.run_auto_install=false\nlockfile=false\n[tasks.compile]\nrun='cargo build --offline --lib --target wasm32-unknown-unknown -p map-core-wasm'\n").unwrap();
    crate::test_support::mise_task::mise_run_without_lock(root, "compile")
        .env("PATH", path)
        .env("RUSTC", rustc)
        .env("CARGO_TARGET_DIR", root.join("target"))
        .env("CARGO_BUILD_JOBS", "1")
        .env("CARGO_INCREMENTAL", "0")
        .env("CARGO_PROFILE_DEV_DEBUG", "0")
        .env("CARGO_PROFILE_DEV_OPT_LEVEL", "0")
        .env("CARGO_NET_OFFLINE", "true")
        .env("CARGO_ENCODED_RUSTFLAGS", "")
        .env("RUSTC_WRAPPER", "")
        .env("RUSTC_WORKSPACE_WRAPPER", "")
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_BUILD_TARGET")
        .output()
        .expect("execute the isolated offline WASM compilation task")
}

fn committed_channel() -> String {
    let config: toml::Value = toml::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/rust-toolchain.toml"
    )))
    .unwrap();
    config["toolchain"]["channel"]
        .as_str()
        .expect("committed Rust channel")
        .to_string()
}

fn installed_tool(channel: &str, tool: &str, root: &Path) -> PathBuf {
    let output = Command::new("rustup")
        .args(["which", "--toolchain", channel, tool])
        .current_dir(root)
        .output()
        .expect("query already-installed committed Rust tool");
    assert!(
        output.status.success(),
        "installed {channel} {tool} required; no install attempted: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let path = PathBuf::from(String::from_utf8(output.stdout).unwrap().trim());
    assert!(path.is_absolute() && path.is_file());
    path
}
