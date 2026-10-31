use crate::codegen::naming::csharp_type_name;
use crate::core::config::Language;
use crate::core::ir::{ApiSurface, ErrorDef};
use heck::ToPascalCase;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) type FamilyInterfaceNames = BTreeMap<String, String>;

pub(crate) fn default_interface_names(errors: &[ErrorDef], naming: impl Fn(&str) -> String) -> FamilyInterfaceNames {
    errors
        .iter()
        .map(|error| (error.name.clone(), naming(&error.name)))
        .collect()
}

pub(crate) fn interface_name(names: &FamilyInterfaceNames, family: &str) -> String {
    names
        .get(family)
        .expect("every declared error family has an allocated interface")
        .clone()
}

pub(crate) fn family_interface_names_with_reserved(
    api: &ApiSurface,
    naming: impl Fn(&str) -> String,
    language: Language,
    reserved_symbols: &[String],
) -> FamilyInterfaceNames {
    let normalize = |name: &str| {
        if language == Language::Php {
            name.to_lowercase()
        } else {
            name.into()
        }
    };
    let mut occupied = occupied_names(api, language)
        .into_iter()
        .chain(reserved_symbols.iter().cloned())
        .map(|name| normalize(&name))
        .collect::<BTreeSet<_>>();
    let defaults = default_interface_names(&api.errors, naming);
    let reserved = defaults.values().map(|name| normalize(name)).collect::<BTreeSet<_>>();
    defaults
        .into_iter()
        .map(|(family, preferred)| {
            let mut name = preferred.clone();
            let mut suffix = 1;
            while occupied.contains(&normalize(&name)) {
                name = if suffix == 1 {
                    format!("{preferred}Family")
                } else {
                    format!("{preferred}Family{suffix}")
                };
                suffix += 1;
                while reserved.contains(&normalize(&name)) || occupied.contains(&normalize(&name)) {
                    name = format!("{preferred}Family{suffix}");
                    suffix += 1;
                }
            }
            occupied.insert(normalize(&name));
            (family, name)
        })
        .collect()
}

fn occupied_names(api: &ApiSurface, language: Language) -> Vec<String> {
    let mut names = Vec::new();
    for typ in &api.types {
        names.push(typ.name.clone());
        match language {
            Language::Csharp => names.push(csharp_type_name(&typ.name)),
            Language::Php => {
                reserve_php_companion(&mut names, &typ.name);
            }
            _ => {}
        }
        if typ.is_trait {
            reserve_trait_names(&mut names, &typ.name, language);
        }
    }
    for item in &api.enums {
        names.push(item.name.clone());
        match language {
            Language::Csharp => names.push(csharp_type_name(&item.name)),
            Language::Php => reserve_php_companion(&mut names, &item.name),
            _ => {}
        }
    }
    for error in &api.errors {
        names.extend([error.name.clone(), format!("{}Exception", error.name)]);
        if language == Language::Php {
            reserve_php_companion(&mut names, &format!("Php{}", error.name));
        }
        for variant in &error.variants {
            if language == Language::Php {
                reserve_php_companion(
                    &mut names,
                    &format!("Php{}", super::python_exception_name(&variant.name, &error.name)),
                );
            }
            names.extend([
                format!("{}Exception", variant.name),
                super::python_exception_name(&variant.name, &error.name),
            ]);
        }
    }
    names
}

fn reserve_php_companion(names: &mut Vec<String>, rust_symbol: &str) {
    // ext-php-rs #[php_interface] emits PhpInterface{Name} in the Rust type namespace.
    if let Some(marker) = rust_symbol.strip_prefix("PhpInterface") {
        names.push(marker.into());
    }
}

fn reserve_trait_names(names: &mut Vec<String>, name: &str, language: Language) {
    match language {
        Language::Csharp => {
            let projected = csharp_type_name(name);
            names.extend([
                format!("I{projected}"),
                format!("{projected}Bridge"),
                format!("{projected}Registry"),
                format!("_{projected}BridgeAdapter"),
            ]);
        }
        Language::Java => {
            let projected = name.to_pascal_case();
            names.extend([format!("I{name}"), format!("{name}Bridge"), format!("{name}Adapter")]);
            names.extend([
                format!("I{projected}"),
                format!("{projected}Bridge"),
                format!("{projected}Adapter"),
            ]);
        }
        Language::Php => {
            names.push(format!("{name}Interface"));
        }
        _ => unreachable!("only C#, Java and PHP use error family interfaces"),
    }
}
