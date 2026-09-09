#![allow(clippy::print_stderr)]

use std::process::{Command, ExitCode};

const SWIFT_FAILURE: u8 = 17;

fn main() -> ExitCode {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    let swift_manifest = args.windows(2).any(|pair| {
        pair[0] == "--manifest-path"
            && pair[1].to_string_lossy().split(['/', '\\']).rev().take(4).eq([
                "Cargo.toml",
                "rust",
                "swift",
                "packages",
            ])
    });
    if swift_manifest {
        let status = if std::env::var("FAIL_SWIFT_POST_BUILD").as_deref() == Ok("1") {
            SWIFT_FAILURE
        } else {
            0
        };
        eprintln!("atomicity fixture cargo: Swift post-build exit {status}");
        return ExitCode::from(status);
    }
    let cargo = std::env::var_os("REAL_CARGO").expect("fixture real Cargo path");
    let status = Command::new(cargo).args(args).status().expect("delegate to real Cargo");
    ExitCode::from(u8::try_from(status.code().unwrap_or(1)).unwrap_or(1))
}
