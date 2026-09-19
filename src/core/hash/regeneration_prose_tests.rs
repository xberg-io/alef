//! Regeneration advice alone does not grant ownership of a handwritten file.

use super::{content_has_alef_marker, extract_hash, inject_hash_line, inject_stamp_line, strip_hash_line};

#[test]
fn regeneration_prose_is_not_an_ownership_marker() {
    for content in [
        "# Generated conformance suites\n\nRegenerate with `alef e2e generate`; edit the fixtures or generator, never the\ngenerated suites.\n",
        "Regenerate with `alef generate`. DO NOT EDIT the resulting output.\n",
        "// Regenerate with `alef generate` when testing this example.\n",
    ] {
        assert!(!content_has_alef_marker(content), "{content}");
        assert_eq!(inject_hash_line(content, "abc123"), content);
        assert_eq!(inject_stamp_line(content, "handle-abi", "registry"), content);
    }
}

#[test]
fn regeneration_comment_headers_keep_their_hash_round_trip() {
    for (prefix, suffix) in [
        ("//", ""),
        ("#", ""),
        ("/*", " */"),
        ("*", ""),
        ("<!--", " -->"),
        (";", ""),
    ] {
        let content = format!("  {prefix} Do NOT edit — ReGenerate with `ALEF generate`.{suffix}\nbody\n");
        assert!(content_has_alef_marker(&content), "{content}");
        let stamped = inject_hash_line(&content, "abc123");
        assert_eq!(extract_hash(&stamped).as_deref(), Some("abc123"), "{stamped}");
        assert_eq!(strip_hash_line(&stamped), content);
    }
}

#[test]
fn negated_regeneration_advice_is_not_an_ownership_marker() {
    for cue in ["not", "NOT", "never", "Never", "isn't", "wasn't"] {
        let content = format!("// Do not edit this example; {cue} regenerate with `alef generate`.\n");
        assert!(!content_has_alef_marker(&content), "{content}");
        assert_eq!(inject_hash_line(&content, "abc123"), content);
    }
}
