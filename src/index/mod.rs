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
