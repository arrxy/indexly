use std::collections::HashMap;

pub struct Lexicon {
    pub terms: HashMap<String, LexiconEntry>,
}

pub struct LexiconEntry {
    pub offset: usize,
    pub length: usize,
    pub doc_freq: usize,
}