use std::collections::{BTreeMap, HashSet};

use crate::core::ir::{ApiSurface, ErrorDef};

#[derive(Clone, Copy)]
pub enum ExceptionNaming {
    Python,
    Nominal,
}

/// One error with shared classes assigned to their most specific compatible family. ~keep
pub struct ExceptionFamily {
    pub error: ErrorDef,
    pub parent: Option<String>,
}

/// Reject unknown families and cycles before emitting any binding artifact. ~keep
pub fn validate_exception_hierarchy(api: &ApiSurface) -> anyhow::Result<()> {
    let parents = exception_parents(api);
    for (child, parent) in &parents {
        anyhow::ensure!(
            api.errors.iter().any(|error| error.name == *parent),
            "unknown exception parent {parent}"
        );
        ancestors(child, &parents)?;
    }
    if !parents.is_empty() {
        variant_owners(api, ExceptionNaming::Python)?;
        variant_owners(api, ExceptionNaming::Nominal)?;
    }
    for naming in [ExceptionNaming::Python, ExceptionNaming::Nominal] {
        for (class, members) in shared_exception_classes(&api.errors, naming) {
            shared_introspection(&api.errors, &class, &members)?;
        }
    }
    Ok(())
}

/// The introspection methods of a class shared by unrelated families: every declaring family's
/// methods by name, first declaration first, each with whether some declaring family lacks it.
/// Bindings carry such a method as a nullable value the families without it leave unset. Two
/// families returning different types from one method name is an error.
pub fn shared_introspection(
    errors: &[ErrorDef],
    class: &str,
    members: &ClassFamilies,
) -> anyhow::Result<Vec<(crate::core::ir::MethodDef, bool)>> {
    let declaring: Vec<&ErrorDef> = errors
        .iter()
        .filter(|error| members.declared_by.contains(&error.name))
        .collect();
    let mut union: Vec<(&crate::core::ir::MethodDef, &str)> = Vec::new();
    for error in &declaring {
        for method in lineage_methods(errors, &error.name) {
            match union.iter().find(|(existing, _)| existing.name == method.name) {
                Some((existing, first)) => anyhow::ensure!(
                    existing.return_type == method.return_type,
                    "incompatible exception introspection {} of shared class {class}: {first} and {} return \
                     different types",
                    method.name,
                    error.name
                ),
                None => union.push((method, &error.name)),
            }
        }
    }
    Ok(union
        .into_iter()
        .map(|(method, _)| {
            let everywhere = declaring.iter().all(|error| {
                lineage_methods(errors, &error.name)
                    .iter()
                    .any(|declared| declared.name == method.name)
            });
            (method.clone(), !everywhere)
        })
        .collect())
}

fn ancestors<'a>(name: &'a str, parents: &'a BTreeMap<String, String>) -> anyhow::Result<Vec<&'a str>> {
    let mut visited = HashSet::new();
    let mut names = Vec::new();
    let mut current = name;
    loop {
        anyhow::ensure!(visited.insert(current), "exception hierarchy cycle at {current}");
        names.push(current);
        let Some(parent) = parents.get(current) else {
            return Ok(names);
        };
        current = parent;
    }
}

/// Preserve legacy first-wins output unless a consumer explicitly declares a hierarchy. ~keep
pub fn exception_families(api: &ApiSurface, naming: ExceptionNaming) -> anyhow::Result<Vec<ExceptionFamily>> {
    validate_exception_hierarchy(api)?;
    let parents = exception_parents(api);
    if parents.is_empty() {
        return Ok(api
            .errors
            .iter()
            .cloned()
            .map(|error| ExceptionFamily { error, parent: None })
            .collect());
    }
    let owners = variant_owners(api, naming)?;
    let mut errors = api
        .errors
        .iter()
        .map(|error| Ok((ancestors(&error.name, &parents)?.len(), error.clone())))
        .collect::<anyhow::Result<Vec<_>>>()?;
    errors.sort_by_key(|(depth, _)| *depth);
    Ok(errors
        .into_iter()
        .map(|(_, mut error)| {
            error
                .variants
                .retain(|variant| owners[&class_name(&variant.name, &error.name, naming)] == error.name);
            ExceptionFamily {
                parent: parents.get(&error.name).cloned(),
                error,
            }
        })
        .collect())
}

fn class_name(variant: &str, error: &str, naming: ExceptionNaming) -> String {
    if matches!(naming, ExceptionNaming::Python) {
        super::python_exception_name(variant, error)
    } else {
        format!("{variant}Exception")
    }
}

fn variant_owners(api: &ApiSurface, naming: ExceptionNaming) -> anyhow::Result<BTreeMap<String, String>> {
    let families = class_families(&api.errors, naming);
    for (class, members) in &families {
        let owner = &members.families[0];
        let owner_error = api
            .errors
            .iter()
            .find(|error| error.name == *owner)
            .ok_or_else(|| anyhow::anyhow!("missing exception owner {owner} for shared class {class}"))?;
        let layout = |error: &ErrorDef| {
            error
                .methods
                .iter()
                .map(|method| (method.name.clone(), method.return_type.clone()))
                .collect::<Vec<_>>()
        };
        for error in api
            .errors
            .iter()
            .filter(|error| members.declared_by.contains(&error.name))
            .filter(|error| same_exception_lineage(&api.errors, &error.name, owner))
        {
            anyhow::ensure!(
                layout(error) == layout(owner_error),
                "incompatible exception introspection for {class} between {owner} and {}",
                error.name
            );
        }
    }
    Ok(families
        .into_iter()
        .map(|(class, members)| (class, members.families[0].clone()))
        .collect())
}

/// The families one variant class belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassFamilies {
    /// Every family with a variant of this class, in declaration order.
    pub declared_by: Vec<String>,
    /// The declaring families that no other declaring family descends from, in declaration
    /// order. The first is the class's base; the class also belongs to the others.
    pub families: Vec<String>,
}

fn class_families(errors: &[ErrorDef], naming: ExceptionNaming) -> BTreeMap<String, ClassFamilies> {
    let parents = exception_parents_of(errors);
    let mut declared: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for error in errors {
        for variant in &error.variants {
            let owners = declared
                .entry(class_name(&variant.name, &error.name, naming))
                .or_default();
            if !owners.contains(&error.name) {
                owners.push(error.name.clone());
            }
        }
    }
    declared
        .into_iter()
        .map(|(class, declared_by)| {
            let families = declared_by
                .iter()
                .filter(|family| {
                    !declared_by
                        .iter()
                        .any(|other| other != *family && lineage(other, &parents).contains(&family.as_str()))
                })
                .cloned()
                .collect();
            (class, ClassFamilies { declared_by, families })
        })
        .collect()
}

/// The variant classes that belong to more than one unrelated family, by class name.
///
/// When unrelated families declare variants of one name, bindings keep a single class under that
/// name. It extends the first family, as it always has, and also counts as a member of every other
/// family: Python lists those families as further bases, Ruby includes a module per family, and
/// PHP, C# and Java implement each family's marker interface. Families related through
/// `exception_base` collapse to the most specific one, which already inherits the others.
pub fn shared_exception_classes(errors: &[ErrorDef], naming: ExceptionNaming) -> BTreeMap<String, ClassFamilies> {
    class_families(errors, naming)
        .into_iter()
        .filter(|(_, members)| members.families.len() > 1)
        .collect()
}

/// The families other than its base that the class of `error::variant` also belongs to.
pub fn additional_exception_families(
    errors: &[ErrorDef],
    error: &str,
    variant: &str,
    naming: ExceptionNaming,
) -> Vec<String> {
    let class = class_name(variant, error, naming);
    shared_exception_classes(errors, naming)
        .remove(&class)
        .map(|members| members.families.into_iter().skip(1).collect())
        .unwrap_or_default()
}

/// `name` followed by its `exception_base` ancestors. A cycle, which
/// [`validate_exception_hierarchy`] rejects, only ends the walk here.
fn lineage<'a>(name: &'a str, parents: &'a BTreeMap<String, String>) -> Vec<&'a str> {
    let mut names = Vec::new();
    let mut current = name;
    while !names.contains(&current) {
        names.push(current);
        match parents.get(current) {
            Some(parent) => current = parent,
            None => break,
        }
    }
    names
}

/// Reject a family marker interface whose name another generated type already takes: a type, enum
/// or error of the API, an exception class, or the interface a trait generates (`I{Trait}` in C#
/// and Java, `{Trait}Interface` in PHP). Two declarations of one name would overwrite each other's
/// file or fail to compile, so the clash is reported at generation instead. `interface_name` names
/// the marker interface of a family in the target language.
pub fn validate_family_interface_names(
    api: &ApiSurface,
    language: &str,
    interface_name: impl Fn(&str) -> String,
) -> anyhow::Result<()> {
    let mut taken: Vec<(String, String)> = Vec::new();
    for typ in &api.types {
        taken.push((typ.name.clone(), format!("type {}", typ.name)));
        if typ.is_trait {
            taken.push((format!("I{}", typ.name), format!("interface of trait {}", typ.name)));
            taken.push((
                format!("{}Interface", typ.name),
                format!("interface of trait {}", typ.name),
            ));
        }
    }
    for enum_def in &api.enums {
        taken.push((enum_def.name.clone(), format!("enum {}", enum_def.name)));
    }
    for error in &api.errors {
        taken.push((error.name.clone(), format!("error {}", error.name)));
        taken.push((
            format!("{}Exception", error.name),
            format!("exception class of {}", error.name),
        ));
        for variant in &error.variants {
            for class in [
                class_name(&variant.name, &error.name, ExceptionNaming::Python),
                class_name(&variant.name, &error.name, ExceptionNaming::Nominal),
            ] {
                taken.push((class, format!("exception class of {}::{}", error.name, variant.name)));
            }
        }
    }
    for error in &api.errors {
        let name = interface_name(&error.name);
        if let Some((_, owner)) = taken.iter().find(|(taken_name, _)| *taken_name == name) {
            anyhow::bail!(
                "{language} marker interface {name} of error family {} collides with the generated {owner}",
                error.name
            );
        }
    }
    Ok(())
}

/// The introspection methods an instance of `family` has: its own, then those it inherits
/// through `exception_base`, nearest first, each name once.
pub fn lineage_methods<'a>(errors: &'a [ErrorDef], family: &str) -> Vec<&'a crate::core::ir::MethodDef> {
    let mut methods: Vec<&crate::core::ir::MethodDef> = Vec::new();
    for member in exception_lineage(errors, family) {
        for method in errors
            .iter()
            .filter(|error| error.name == member)
            .flat_map(|error| &error.methods)
        {
            if methods.iter().all(|existing| existing.name != method.name) {
                methods.push(method);
            }
        }
    }
    methods
}

/// `family` followed by its `exception_base` ancestors, nearest first.
pub fn exception_lineage(errors: &[ErrorDef], family: &str) -> Vec<String> {
    let parents = exception_parents_of(errors);
    lineage(family, &parents).into_iter().map(str::to_string).collect()
}

/// Whether `a` and `b` are the same family or one descends from the other through `exception_base`.
pub fn same_exception_lineage(errors: &[ErrorDef], a: &str, b: &str) -> bool {
    let parents = exception_parents_of(errors);
    lineage(a, &parents).contains(&b) || lineage(b, &parents).contains(&a)
}

fn exception_parents(api: &ApiSurface) -> BTreeMap<String, String> {
    exception_parents_of(&api.errors)
}

fn exception_parents_of(errors: &[ErrorDef]) -> BTreeMap<String, String> {
    errors
        .iter()
        .filter_map(|error| {
            error
                .exception_base
                .as_ref()
                .map(|parent| (error.name.clone(), parent.clone()))
        })
        .collect()
}
