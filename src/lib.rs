//! Disk-backed passage search: TSV input -> sorted runs -> compressed index -> BM25 results.
//!
//! Indexing assigns dense internal document IDs in input order. The page table
//! maps those IDs back to the dataset's passage IDs and original text offsets.
//! Readers trust files produced together by this pipeline; use the same tokenizer
//! and unchanged source TSV when searching an index.

pub mod app;
mod binary;
pub mod compression;
pub mod index;
pub mod indexer;
pub mod merger;
pub mod retrieval;
pub mod tokenizer;
