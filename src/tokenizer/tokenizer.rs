//! Tokenization contract: indexing and retrieval must normalize text consistently.

/// Produces normalized tokens in source order, including repeated occurrences.
pub trait Tokenizer {
    /// Return owned tokens so a builder can retain terms after the input text is freed.
    fn tokenize(&self, text: &str) -> Vec<String>;
}
