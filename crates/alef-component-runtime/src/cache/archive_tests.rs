//! Archive shapes `extract_safely` must refuse: every one of them is something
//! `alef component package` never writes.

use super::tests::manifest_declaring_library_size;
use super::*;
use std::io::Cursor;

/// One raw tar entry: its name exactly as written into the header (no validation), its type and
/// its contents.
struct RawEntry<'a> {
    name: &'a [u8],
    kind: tar::EntryType,
    link: Option<&'a [u8]>,
    content: &'a [u8],
}

fn file<'a>(name: &'a str, content: &'a [u8]) -> RawEntry<'a> {
    RawEntry {
        name: name.as_bytes(),
        kind: tar::EntryType::Regular,
        link: None,
        content,
    }
}

/// A gzip'd tar of `entries`, with names written straight into the header bytes so paths the
/// `tar` builder would refuse (absolute, traversal) can still be produced.
fn archive(entries: &[RawEntry<'_>]) -> Vec<u8> {
    let mut tar_bytes = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut tar_bytes);
        for entry in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(entry.content.len() as u64);
            header.set_mode(0o644);
            header.set_entry_type(entry.kind);
            header.as_mut_bytes()[..entry.name.len()].copy_from_slice(entry.name);
            if let Some(link) = entry.link {
                header.as_mut_bytes()[157..157 + link.len()].copy_from_slice(link);
            }
            header.set_cksum();
            builder.append(&header, Cursor::new(entry.content)).unwrap();
        }
        builder.finish().unwrap();
    }
    let mut compressed = Vec::new();
    let mut encoder = flate2::write::GzEncoder::new(&mut compressed, flate2::Compression::default());
    encoder.write_all(&tar_bytes).unwrap();
    encoder.finish().unwrap();
    compressed
}

fn manifest_bytes() -> Vec<u8> {
    manifest_declaring_library_size(6).canonical_bytes().unwrap()
}

fn extract(entries: &[RawEntry<'_>]) -> (tempfile::TempDir, Result<(), ComponentError>) {
    let dir = tempfile::tempdir().unwrap();
    let result = extract_safely(Cursor::new(archive(entries)), dir.path());
    (dir, result)
}

#[test]
fn a_well_formed_archive_extracts() {
    let manifest = manifest_bytes();
    let (dir, result) = extract(&[file("component.json", &manifest), file("libdemo.so", b"native")]);
    result.unwrap();
    assert_eq!(fs::read(dir.path().join("libdemo.so")).unwrap(), b"native");
}

#[test]
fn rejects_an_archive_whose_first_entry_is_not_the_manifest() {
    let manifest = manifest_bytes();
    let (dir, result) = extract(&[file("libdemo.so", b"native"), file("component.json", &manifest)]);
    assert!(matches!(result, Err(ComponentError::UnsafeArchiveEntry(path)) if path == "libdemo.so"));
    assert!(!dir.path().join("libdemo.so").exists());
}

#[test]
fn rejects_an_absolute_entry_path() {
    let manifest = manifest_bytes();
    let (_dir, result) = extract(&[file("component.json", &manifest), file("/tmp/libdemo.so", b"native")]);
    assert!(matches!(result, Err(ComponentError::UnsafeArchiveEntry(path)) if path == "/tmp/libdemo.so"));
}

#[test]
fn rejects_a_hard_link_library_entry() {
    let manifest = manifest_bytes();
    let (dir, result) = extract(&[
        file("component.json", &manifest),
        RawEntry {
            name: b"libdemo.so",
            kind: tar::EntryType::Link,
            link: Some(b"component.json"),
            content: b"",
        },
    ]);
    assert!(matches!(result, Err(ComponentError::UnsafeArchiveEntry(path)) if path == "libdemo.so"));
    assert!(!dir.path().join("libdemo.so").exists());
}

#[test]
fn rejects_a_duplicate_library_entry() {
    let manifest = manifest_bytes();
    let (dir, result) = extract(&[
        file("component.json", &manifest),
        file("libdemo.so", b"native"),
        file("libdemo.so", b"evil!!"),
    ]);
    assert!(matches!(result, Err(ComponentError::UnsafeArchiveEntry(path)) if path == "libdemo.so"));
    assert_eq!(
        fs::read(dir.path().join("libdemo.so")).unwrap(),
        b"native",
        "the second copy must never overwrite the first"
    );
}

/// Regression: a manifest naming its library `component.json` made extraction overwrite the
/// manifest with the library bytes. Rejected now before any library byte is written, whatever the
/// letter case (case-insensitive file systems).
#[test]
fn rejects_a_library_named_after_the_archive_metadata() {
    for name in ["component.json", "Component.JSON", "component.sig"] {
        let mut manifest = manifest_declaring_library_size(6);
        manifest.library.file = name.to_owned();
        let manifest = manifest.canonical_bytes().unwrap();
        let (dir, result) = extract(&[file("component.json", &manifest), file(name, b"native")]);
        assert!(
            matches!(result, Err(ComponentError::UnsafeLibraryPath(ref path)) if path == name),
            "{name}: {result:?}"
        );
        assert_eq!(fs::read(dir.path().join("component.json")).unwrap(), manifest, "{name}");
    }
}
