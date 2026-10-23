//! Aho-Corasick finds query terms; a fixed window around the first hit supplies context.
//! Whole-token Unicode normalization follows indexing; brackets preserve source spelling.

use aho_corasick::{AhoCorasick, MatchKind};

const WINDOW: usize = 240;
const CONTEXT: usize = 60;

/// Build once per query and reuse for the retained passages.
pub struct SnippetGenerator {
    matcher: AhoCorasick,
}

impl SnippetGenerator {
    pub fn new(terms: &[String]) -> Self {
        Self {
            matcher: AhoCorasick::builder()
                .match_kind(MatchKind::LeftmostLongest)
                .build(terms.iter().filter(|term| !term.is_empty()))
                .unwrap(),
        }
    }

    /// Show up to 240 characters around the first matching token, or the text's start.
    pub fn generate(&self, text: &str) -> String {
        let text = text.replace(|c: char| c.is_control(), " ");
        let mut matches = Vec::new();
        let mut offset = 0;
        for piece in text.split_inclusive(|c: char| !c.is_alphanumeric()) {
            let token = piece.trim_end_matches(|c: char| !c.is_alphanumeric());
            let normalized = token.to_lowercase();
            if self
                .matcher
                .find(&normalized)
                .is_some_and(|m| m.range() == (0..normalized.len()))
            {
                matches.push((offset, offset + token.len()));
            }
            offset += piece.len();
        }
        let first = matches.first().map_or(0, |&(begin, _)| begin);
        let low = text[..first]
            .char_indices()
            .rev()
            .nth(CONTEXT - 1)
            .map_or(0, |(i, _)| i);
        let high = text[low..]
            .char_indices()
            .nth(WINDOW)
            .map_or(text.len(), |(i, _)| low + i);
        let mut result = text[low..high].to_owned();
        // Insert from right to left so earlier match offsets remain valid.
        for (begin, end) in matches.into_iter().rev() {
            if begin >= low && end <= high {
                result.insert(end - low, ']');
                result.insert(begin - low, '[');
            }
        }
        if low > 0 {
            result.insert_str(0, "... ");
        }
        if high < text.len() {
            result.push_str(" ...");
        }
        result.trim().to_owned()
    }
}

#[cfg(test)]
#[path = "../../tests/unit/retrieval/snippet.rs"]
mod tests;
