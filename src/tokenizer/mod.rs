//! Shared tokenization rules for both index construction and query processing.

mod simple;
mod tokenizer;

pub use simple::SimpleTokenizer;
pub use tokenizer::Tokenizer;
