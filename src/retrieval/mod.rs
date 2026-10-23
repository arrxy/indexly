//! Query parsing, exact AND/OR retrieval, BM25 scoring, and result snippets.

mod bm25;
mod cli;
mod dat;
mod snippet;

pub use bm25::Bm25;
pub use cli::{run_cli, run_session};
pub use dat::{QueryMode, SearchEngine, SearchHit, SearchResult, TermScore};
