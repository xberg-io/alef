use super::super::super::internally_tagged_variants::push_owner_segment;
use super::super::super::types::{FieldResolver, JsonNavStep};

/// Appends the [`JsonNavStep::Index`] a segment's trailing `[..]` names, if any, to `steps`.
///
/// `Ok`-shaped as `Option<()>` so [`FieldResolver::swift_json_bridged_navigation`] can use `?` to
/// abort its whole walk the moment one segment's bracket contents are not a plain non-negative
/// integer -- a wildcard `[]` or a string map key `[key]` is a distinct traversal this walk does
/// not attempt to decode generically. A segment with no bracket at all is not an error: it simply
/// contributes no index step. ~keep
pub(super) fn push_numeric_bracket_step(segment: &str, steps: &mut Vec<JsonNavStep>) -> Option<()> {
    let Some(open) = segment.find('[') else {
        return Some(());
    };
    let Some(close) = segment[open..].find(']') else {
        return Some(());
    };
    let inside = &segment[open + 1..open + close];
    let index: usize = inside.parse().ok()?;
    steps.push(JsonNavStep::Index(index));
    Some(())
}

impl FieldResolver {
    /// The `JsonNavStep`s that walk the segments after a JSON-bridged boundary segment.
    pub(super) fn swift_bridged_tail_steps(
        &self,
        boundary: &str,
        later_segments: &[&str],
        owner_path: &mut String,
    ) -> Option<Vec<JsonNavStep>> {
        let mut steps = Vec::new();
        push_numeric_bracket_step(boundary, &mut steps)?;
        for later in later_segments {
            let later_bare = later.split('[').next().unwrap_or(later);
            // ~keep A segment naming a variant of an internally-tagged serde enum is not
            // a JSON key: that wire form is flat, with the variant's own fields beside
            // the discriminator and no key for the variant name. A typed fixture path
            // spells the variant as a segment anyway (`format.excel.sheet_count`), so a
            // literal `JsonNavStep::Key("excel")` would look up a key that does not exist
            // and `JSONSerialization` would return nil for it. Zig's JSON-walking codegen
            // has the identical problem and asks the identical IR-derived question.
            if self.is_internally_tagged_variant_segment(owner_path, later_bare) {
                push_owner_segment(owner_path, later_bare);
                continue;
            }
            steps.push(JsonNavStep::Key(later_bare.to_string()));
            push_numeric_bracket_step(later, &mut steps)?;
            push_owner_segment(owner_path, later_bare);
        }
        Some(steps)
    }
}
