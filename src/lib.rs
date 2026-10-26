//! Indexly is a disk-backed, exact BM25 passage search engine.
//!
//! Start with [`index_documents`] for application-supplied text, or [`build_index`]
//! for an existing `id<TAB>text` TSV. Open the resulting directory with
//! [`SearchEngine::open_default`] and choose [`QueryMode::Conjunctive`] (AND) or
//! [`QueryMode::Disjunctive`] (OR). PDF/DOCX extraction and collection locking belong
//! to the consuming application.
//!
//! ```no_run
//! use indexly::{Document, IndexOptions, QueryMode, SearchEngine, index_documents};
//!
//! # fn main() -> std::io::Result<()> {
//! let documents = [
//!     Document::new(10, "Rust supports fast document search."),
//!     Document::new(20, "Compressed posting lists stay on disk."),
//! ];
//! let report = index_documents(documents, "new-index", &IndexOptions::default())?;
//! let mut engine = SearchEngine::open_default(&report.index_dir)?;
//! let results = engine.search("rust search", QueryMode::Conjunctive)?;
//! for hit in results.hits {
//!     println!("{}: {}", hit.passage_id, engine.passage_text(hit.doc_id)?);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! CPU sorting is the default and has no GPU dependency. Enable `gpu-sort` and
//! explicitly choose [`SortBackend::Gpu`] for experimental Metal sorting.
//! See the repository's `docs/index-format.md` for persisted-format compatibility.

mod api;
pub use api::{
    BuildReport, Document, IndexOptions, SearchOptions, SortBackend, build_index, index_documents,
};
pub use retrieval::{QueryMode, SearchEngine, SearchHit, SearchResult, TermScore};

// Keep the existing low-level surface available to applications such as Indexify.
// New consumers should use the root-level API above.
#[doc(hidden)]
pub mod app;
#[doc(hidden)]
pub mod compression;
#[doc(hidden)]
pub mod index;
#[doc(hidden)]
pub mod indexer;
#[doc(hidden)]
pub mod merger;
#[doc(hidden)]
pub mod retrieval;
#[doc(hidden)]
pub mod tokenizer;
