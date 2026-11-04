use crate::core::ir::{ApiSurface, EnumDef, ErrorDef, FunctionDef, TypeDef};

#[derive(Clone, Copy)]
pub(super) struct FixtureApi<'a> {
    pub(super) type_defs: &'a [TypeDef],
    pub(super) enums: &'a [EnumDef],
    pub(super) functions: &'a [FunctionDef],
    pub(super) errors: &'a [ErrorDef],
    pub(super) non_python: Option<&'a ApiSurface>,
}

impl<'a> FixtureApi<'a> {
    pub(super) fn new(
        type_defs: &'a [TypeDef],
        enums: &'a [EnumDef],
        functions: &'a [FunctionDef],
        errors: &'a [ErrorDef],
    ) -> Self {
        Self {
            type_defs,
            enums,
            functions,
            errors,
            non_python: None,
        }
    }

    pub(super) fn from_surface(api: &'a ApiSurface, non_python: Option<&'a ApiSurface>) -> Self {
        Self {
            non_python,
            ..Self::new(&api.types, &api.enums, &api.functions, &api.errors)
        }
    }

    pub(super) fn projected(self) -> Option<Self> {
        self.non_python.map(|api| Self {
            type_defs: &api.types,
            enums: &api.enums,
            ..self
        })
    }

    pub(super) fn for_language(self, language: &str) -> Self {
        if uses_projected_fixture_api(language) {
            self.projected().unwrap_or(self)
        } else {
            self
        }
    }
}

pub(super) fn uses_projected_fixture_api(language: &str) -> bool {
    !matches!(language, "python" | "rust")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_fixtures_use_projection_while_rust_and_python_keep_canonical_types() {
        let canonical = ApiSurface {
            types: vec![TypeDef {
                name: "Counter".into(),
                ..Default::default()
            }],
            functions: vec![FunctionDef {
                name: "register_counter".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let projected = ApiSurface {
            types: vec![TypeDef {
                name: "HostCounter".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let api = FixtureApi::from_surface(&canonical, Some(&projected));
        for language in ["rust", "python"] {
            assert_eq!(api.for_language(language).type_defs[0].name, "Counter");
        }
        for language in ["dart", "swift", "kotlin_android"] {
            let view = api.for_language(language);
            assert_eq!(view.type_defs[0].name, "HostCounter");
            assert_eq!(view.functions[0].name, "register_counter");
        }
        assert_eq!(canonical.types[0].name, "Counter");
    }
}
