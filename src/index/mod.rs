//! Persistent index components and their query-time readers.
//! The lexicon locates each term's postings and block directory. The page table
//! maps internal document IDs to passage IDs, lengths, and source-text offsets.
//! Collection statistics provide the corpus totals used by BM25.

mod lexicon;
mod metadata;
mod page_table;
mod passages;
mod postings_reader;
pub use passages::{PassageStore, prepare_passages};
pub use postings_reader::PostingsReader;

pub use lexicon::{Lexicon, LexiconEntry};
pub use metadata::CollectionStats;
pub use page_table::{PageTable, PageTableEntry, PassageSource};
