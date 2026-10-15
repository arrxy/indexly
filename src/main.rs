mod index;
mod indexer;
mod tokenizer;

use std::io;

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() != 4 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Usage: search-engine <dataset.tsv> <output_dir> <max_postings>",
        ));
    }

    let max_postings = args[3].parse::<usize>().map_err(|error| {
        io::Error::new(io::ErrorKind::InvalidInput, error)
    })?;

    let tokenizer = tokenizer::SimpleTokenizer;

    let runs = indexer::build_runs(
        &args[1],
        &args[2],
        max_postings,
        &tokenizer,
    )?;

    println!("Wrote {runs} sorted run files.");
    Ok(())
}