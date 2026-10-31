use std::collections::HashSet;
use std::sync::Arc;

pub(super) trait OptionalFieldSet {
    fn contains(&self, name: &str) -> bool;
}

impl OptionalFieldSet for HashSet<String> {
    fn contains(&self, name: &str) -> bool {
        HashSet::contains(self, name)
    }
}

#[derive(Clone, Default)]
pub(super) struct OptionalFields {
    shared: Arc<HashSet<String>>,
    local: HashSet<String>,
}

impl From<HashSet<String>> for OptionalFields {
    fn from(local: HashSet<String>) -> Self {
        Self {
            shared: Arc::default(),
            local,
        }
    }
}

impl OptionalFields {
    pub(super) fn contains(&self, name: &str) -> bool {
        self.shared.contains(name) || self.local.contains(name)
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = &String> {
        self.shared
            .iter()
            .chain(self.local.iter().filter(|name| !self.shared.contains(*name)))
    }

    pub(super) fn insert(&mut self, name: String) {
        if !self.shared.contains(&name) {
            self.local.insert(name);
        }
    }

    pub(super) fn extend(&mut self, names: HashSet<String>) {
        self.include_shared(Arc::new(names));
    }

    pub(super) fn include_shared(&mut self, names: Arc<HashSet<String>>) {
        if self.shared.is_empty() {
            self.shared = names;
        } else if !Arc::ptr_eq(&self.shared, &names) {
            self.shared = Arc::new(self.shared.iter().chain(names.iter()).cloned().collect());
        }
    }
}

impl OptionalFieldSet for OptionalFields {
    fn contains(&self, name: &str) -> bool {
        self.contains(name)
    }
}
