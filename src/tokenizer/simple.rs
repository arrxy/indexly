//! Split on non-alphanumeric Unicode characters, discard empties, and lowercase.
//! No stemming, stop-word removal, or accent folding is performed.

use crate::tokenizer::tokenizer::Tokenizer;
/// The normalizer used for both indexed passages and queries.
pub struct SimpleTokenizer;

impl Tokenizer for SimpleTokenizer {
    fn tokenize(&self, text: &str) -> Vec<String> {
        text.split(|c: char| !c.is_alphanumeric())
            .filter(|term| !term.is_empty())
            .map(|term| term.to_lowercase())
            .collect()
    }
}
