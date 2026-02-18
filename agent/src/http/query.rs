use std::fmt;

pub const MAX_PAGE_LIMIT: usize = 100;

pub struct Page {
    pub limit: usize,
    pub offset: usize,
}

impl Default for Page {
    fn default() -> Self {
        Self {
            limit: 10,
            offset: 0,
        }
    }
}

pub struct QueryParams {
    pairs: Vec<(&'static str, String)>,
}

impl QueryParams {
    pub fn new() -> Self {
        Self { pairs: Vec::new() }
    }

    pub fn add(mut self, key: &'static str, value: &str) -> Self {
        self.pairs.push((key, value.to_string()));
        self
    }

    pub fn expand<I>(mut self, expansions: I) -> Self
    where
        I: IntoIterator,
        I::Item: fmt::Display,
    {
        for expansion in expansions {
            self.pairs.push(("expand", expansion.to_string()));
        }
        self
    }

    pub fn paginate(mut self, page: &Page) -> Self {
        self.pairs.push(("limit", page.limit.to_string()));
        self.pairs.push(("offset", page.offset.to_string()));
        self
    }

    pub fn into_pairs(self) -> Vec<(String, String)> {
        self.pairs
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect()
    }
}
