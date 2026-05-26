use std::collections::HashMap;

/// Stores open document contents keyed by URI.
#[derive(Default)]
pub struct DocumentStore {
    docs: HashMap<String, String>,
}

impl DocumentStore {
    pub fn open(&mut self, uri: String, content: String) {
        self.docs.insert(uri, content);
    }

    pub fn update(&mut self, uri: &str, content: String) {
        self.docs.insert(uri.to_string(), content);
    }

    pub fn close(&mut self, uri: &str) {
        self.docs.remove(uri);
    }

    pub fn get(&self, uri: &str) -> Option<&str> {
        self.docs.get(uri).map(|s| s.as_str())
    }
}
