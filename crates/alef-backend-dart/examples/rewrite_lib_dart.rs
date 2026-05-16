//! One-shot CLI to apply [`alef_backend_dart::rewrite_frb_sealed_variants`] to
//! one or more frb-generated `lib.dart` files in-place. Useful when the alef
//! build pipeline is bypassed (e.g. running `flutter_rust_bridge_codegen` via
//! `task dart:codegen` directly).
//!
//! Usage:
//!
//! ```sh
//! cargo run -p alef-backend-dart --example rewrite_lib_dart -- path/to/lib.dart [path/to/other.dart ...]
//! ```

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let paths: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();
    if paths.is_empty() {
        eprintln!("usage: rewrite_lib_dart <path1> [<path2> ...]");
        std::process::exit(2);
    }
    for path in &paths {
        let content = std::fs::read_to_string(path)?;
        let rewritten = alef_backend_dart::rewrite_frb_sealed_variants(&content);
        if rewritten != content {
            std::fs::write(path, &rewritten)?;
            println!("rewrote: {}", path.display());
        } else {
            println!("unchanged: {}", path.display());
        }
    }
    Ok(())
}
