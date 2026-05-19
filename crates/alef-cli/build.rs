// Windows MSVC defaults to a 1 MB thread stack which is too small for clap's
// `--help` recursion across alef's large nested command tree (`alef generate
// --help` / `alef all --help` stack-overflow in CI). Match Linux/macOS by
// raising it to 8 MB. The flag is MSVC-only; cargo:rustc-link-arg is picked
// up regardless of the RUSTFLAGS env var the CI runner sets.
fn main() {
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.contains("windows-msvc") {
        println!("cargo:rustc-link-arg-bin=alef=/STACK:8388608");
    }
}
