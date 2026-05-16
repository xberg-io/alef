//! No-op validators for "documentation" languages — `text`, `mermaid`, `powershell`, `xml`, `docker`.
//!
//! These languages either lack a viable syntax checker without heavy external toolchains
//! (mermaid, powershell, xml, docker) or are pure prose (text). Snippets in these fences
//! pass automatically; their value is that the audit subcommand stops reporting them as
//! `UnknownLanguage`. Use a stricter validator (or an external linter) when you really need
//! to verify the contents.

use crate::error::Result;
use crate::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::validators::SnippetValidator;

macro_rules! documentation_validator {
    ($name:ident, $lang:ident) => {
        pub struct $name;

        impl SnippetValidator for $name {
            fn language(&self) -> Language {
                Language::$lang
            }

            fn is_available(&self) -> bool {
                true
            }

            fn validate(
                &self,
                _snippet: &Snippet,
                _level: ValidationLevel,
                _timeout_secs: u64,
            ) -> Result<(SnippetStatus, Option<String>)> {
                Ok((SnippetStatus::Pass, None))
            }

            fn max_level(&self) -> ValidationLevel {
                ValidationLevel::Syntax
            }
        }
    };
}

documentation_validator!(TextValidator, Text);
documentation_validator!(MermaidValidator, Mermaid);
documentation_validator!(PowerShellValidator, PowerShell);
documentation_validator!(XmlValidator, Xml);
documentation_validator!(DockerValidator, Docker);
