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

fn extract(entries: &[RawEntry<'_>]) -> (tempfile::TempDir, Result<(), ComponentError>) {
    let dir = tempfile::tempdir().unwrap();
    let result = extract_safely(Cursor::new(archive(entries)), dir.path());
    (dir, result)
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
