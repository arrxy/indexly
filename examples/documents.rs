//! Run with: cargo run --example documents -- /tmp/my-new-index
use indexly::{Document, IndexOptions, QueryMode, SearchEngine, index_documents};
use std::io;

fn main() -> io::Result<()> {
    let output = std::env::args_os().nth(1).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: documents <new-index-directory>",
        )
    })?;
    let documents = [
        Document::new(10, "Rust supports fast document search."),
        Document::new(20, "Compressed posting lists keep the index on disk."),
        Document::new(30, "Rust powers a terminal document search application."),
    ];
    let report = index_documents(documents, output, &IndexOptions::default())?;
    println!(
        "Indexed {} passages and {} terms",
        report.documents, report.terms
    );
    let mut engine = SearchEngine::open_default(&report.index_dir)?;
    let result = engine.search("rust search", QueryMode::Conjunctive)?;
    println!("{} matches", result.matched_documents);
    for hit in result.hits {
        println!("{}: {:.4} {}", hit.passage_id, hit.score, hit.snippet);
        println!("  {}", engine.passage_text(hit.doc_id)?);
    }
    Ok(())
}
