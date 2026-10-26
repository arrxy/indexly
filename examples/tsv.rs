//! Run with: cargo run --example tsv -- input.tsv /tmp/my-new-index
use indexly::{IndexOptions, QueryMode, SearchOptions, build_index};
use std::io;

fn main() -> io::Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: tsv <input.tsv> <new-index-directory>",
        ));
    }
    let options = IndexOptions {
        threads: 2,
        merge_threads: 1,
        ..IndexOptions::default()
    };
    let report = build_index(&args[0], &args[1], &options)?;
    let mut engine = SearchOptions {
        snippets: false,
        ..SearchOptions::default()
    }
    .open(&report.index_dir)?;
    let result = engine.search("rust search", QueryMode::Disjunctive)?;
    println!(
        "{} indexed passages; {} matches",
        report.documents, result.matched_documents
    );
    for hit in result.hits {
        println!("{}: {:.4}", hit.passage_id, hit.score);
    }
    Ok(())
}
