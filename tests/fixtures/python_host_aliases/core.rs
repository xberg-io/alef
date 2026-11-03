pub type Error = String;

pub trait Counter {
    fn count(&self, text: &str) -> usize;
    fn alternate(&self, text: &str) -> usize;

    fn checked_count(&self, text: &str) -> Result<usize, Error> {
        Ok(self.count(text))
    }

    fn describe(&self, text: &str) -> Result<usize, Error> {
        self.checked_count(text)
    }
}
