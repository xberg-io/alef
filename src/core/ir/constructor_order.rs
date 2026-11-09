use super::TypeDef;

/// True when no parameter without a default follows one with a default, the shape Python,
/// PHP and PyO3 signatures require.
#[must_use]
pub fn defaults_are_trailing(has_default: impl IntoIterator<Item = bool>) -> bool {
    let mut seen_default = false;
    for defaulted in has_default {
        if defaulted {
            seen_default = true;
        } else if seen_default {
            return false;
        }
    }
    true
}

impl TypeDef {
    /// Whether this type is listed in `constructor_declared_order` and has no recorded constructor
    /// history, whose own signature order takes precedence. A backend whose signature can express
    /// any order uses this directly; the others go through [`Self::uses_declared_constructor_order`].
    #[must_use]
    pub fn declared_constructor_order_requested(&self) -> bool {
        self.version.declared_constructor_order && !self.has_recorded_constructors()
    }

    /// Whether a positional constructor lists this type's parameters in declaration order.
    ///
    /// Requires [`Self::declared_constructor_order_requested`]. `has_default` yields
    /// each parameter's defaulted-ness in declaration order; when a defaultless parameter
    /// follows a defaulted one the order is not expressible in the target language, so this
    /// returns `false` and the caller keeps its required-first order.
    #[must_use]
    pub fn uses_declared_constructor_order(&self, backend: &str, has_default: impl IntoIterator<Item = bool>) -> bool {
        if !self.declared_constructor_order_requested() {
            return false;
        }
        if defaults_are_trailing(has_default) {
            return true;
        }
        tracing::warn!(
            r#type = %self.name,
            backend,
            "constructor_declared_order ignored: a required field follows an optional one, so declaration \
             order is not a valid signature; keeping required-first order"
        );
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::VersionAnnotation;

    fn typ(declared: bool, history: Vec<String>) -> TypeDef {
        TypeDef {
            name: "Config".to_string(),
            version: VersionAnnotation {
                declared_constructor_order: declared,
                legacy_constructor_fields: history,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn trailing_defaults_are_valid_and_a_required_after_a_default_is_not() {
        assert!(defaults_are_trailing([]));
        assert!(defaults_are_trailing([false, false, true, true]));
        assert!(defaults_are_trailing([true, true]));
        assert!(!defaults_are_trailing([false, true, false]));
        assert!(!defaults_are_trailing([true, false]));
    }

    #[test]
    fn an_unlisted_type_never_uses_declared_order() {
        assert!(!typ(false, vec![]).uses_declared_constructor_order("test", [false, true]));
    }

    #[test]
    fn a_listed_type_uses_declared_order_only_when_defaults_trail() {
        let listed = typ(true, vec![]);

        assert!(listed.uses_declared_constructor_order("test", [false, true]));
        assert!(!listed.uses_declared_constructor_order("test", [true, false]));
    }

    #[test]
    fn recorded_constructor_history_takes_precedence() {
        assert!(!typ(true, vec!["a".to_string()]).uses_declared_constructor_order("test", [false, true]));
        assert!(!typ(true, vec!["a".to_string()]).declared_constructor_order_requested());
    }
}
