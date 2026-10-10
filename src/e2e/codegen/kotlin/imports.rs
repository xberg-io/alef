//! The import block of a generated Kotlin e2e test file.
//!
//! Every import a test file emits is collected here rather than written straight to the output,
//! so that one funnel owns package qualification and de-duplication for all of them. Emitters
//! that each spelled the qualification themselves is exactly how a configured fully-qualified
//! `class` ended up imported both correctly AND as `<pkg>.<pkg>.<Class>` in the same file — one
//! path split the name, the other prefixed it unconditionally, and the bogus spelling failed the
//! Kotlin compile with an unresolved reference. ~keep

/// Ordered, de-duplicated import paths for one generated test file.
pub(super) struct ImportBlock {
    /// Package the binding's public types live in. Test files live in its `.e2e` child package,
    /// and Kotlin child packages do not see their parent's symbols, so binding types need an
    /// explicit import.
    binding_package: String,
    paths: Vec<String>,
}

impl ImportBlock {
    pub(super) fn new(binding_package: String) -> Self {
        Self {
            binding_package,
            paths: Vec::new(),
        }
    }

    /// Add an already-complete import path — a stdlib, JUnit or Jackson symbol.
    pub(super) fn push(&mut self, path: &str) {
        if !self.paths.iter().any(|existing| existing == path) {
            self.paths.push(path.to_string());
        }
    }

    /// Add a type from the binding package, qualifying it only when it is not already qualified.
    pub(super) fn push_binding_type(&mut self, type_name: &str) {
        let path = crate::codegen::naming::qualified_type_path(&self.binding_package, type_name);
        self.push(&path);
    }

    /// Render the block. Empty when nothing was added.
    pub(super) fn render(&self) -> String {
        crate::e2e::template_env::render(
            "kotlin/test_imports.kt.jinja",
            crate::alef_context! { imports => self.paths },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::ImportBlock;

    const BINDING_PACKAGE: &str = "dev.sample.bindings";

    fn block() -> ImportBlock {
        ImportBlock::new(BINDING_PACKAGE.to_string())
    }

    #[test]
    fn bare_binding_type_is_qualified_with_the_binding_package() {
        let mut imports = block();
        imports.push_binding_type("SampleClient");
        assert_eq!(imports.render(), "import dev.sample.bindings.SampleClient\n");
    }

    /// The defect this module exists to prevent: prefixing a name that already carries the
    /// package produces an unresolvable `dev.sample.bindings.dev.sample.bindings.SampleClient`.
    #[test]
    fn already_qualified_binding_type_is_left_alone() {
        let mut imports = block();
        imports.push_binding_type("dev.sample.bindings.SampleClient");
        assert_eq!(imports.render(), "import dev.sample.bindings.SampleClient\n");
    }

    /// The two spellings of one configured name must collapse to a single import, not two —
    /// which is what the file showed before the qualification was centralized here.
    #[test]
    fn the_two_spellings_of_one_type_collapse_to_a_single_import() {
        let mut imports = block();
        imports.push_binding_type("dev.sample.bindings.SampleClient");
        imports.push_binding_type("SampleClient");
        assert_eq!(imports.render(), "import dev.sample.bindings.SampleClient\n");
    }

    #[test]
    fn insertion_order_is_preserved_and_non_binding_paths_are_untouched() {
        let mut imports = block();
        imports.push("org.junit.jupiter.api.Test");
        imports.push_binding_type("SampleOptions");
        imports.push("kotlin.test.assertEquals");
        assert_eq!(
            imports.render(),
            "import org.junit.jupiter.api.Test\nimport dev.sample.bindings.SampleOptions\nimport kotlin.test.assertEquals\n"
        );
    }

    #[test]
    fn an_empty_block_renders_nothing() {
        assert_eq!(block().render(), "");
    }
}
