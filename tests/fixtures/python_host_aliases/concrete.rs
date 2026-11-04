pub struct Widget {
    pub value: usize,
}

impl Widget {
    pub fn new(value: usize) -> Self {
        Self { value }
    }

    pub fn checked_count(&self, text: &str) -> usize {
        self.value + text.len()
    }
}

pub struct Implementer {
    pub value: usize,
}

impl Implementer {
    pub fn new(value: usize) -> Self {
        Self { value }
    }
}

impl Counter for Implementer {
    fn count(&self, text: &str) -> usize {
        self.value + text.len()
    }

    fn alternate(&self, text: &str) -> usize {
        self.count(text)
    }

    fn checked_count(&self, text: &str) -> Result<usize, Error> {
        Ok(self.count(text))
    }
}
