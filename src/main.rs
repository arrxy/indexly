mod index;
mod indexer;
mod tokenizer;
use std::io;

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.get(1).map(String::as_str) == Some("--inspect") {
        if args.len() != 3 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Usage: search-engine --inspect <run_file>",
            ));
        }

        return indexer::inspect_run(&args[2]);
    }

    if args.len() < 3 || args.len() > 4 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Usage: search-engine <dataset.tsv> <output_dir> [max_batch_bytes]",
        ));
    }

    let tokenizer = tokenizer::SimpleTokenizer;
    let max_batch_bytes = args
        .get(3)
        .map(|value| value.parse::<usize>())
        .transpose()
        .map_err(|error| {
            io::Error::new(io::ErrorKind::InvalidInput, error)
        })?
        .unwrap_or(64 * 1024);
    let runs = indexer::build_runs(
        &args[1],
        &args[2],
        max_batch_bytes,
        &tokenizer,
    )?;

    println!("Wrote {runs} sorted run files.");
    Ok(())
}