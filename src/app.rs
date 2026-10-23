//! Command-line orchestration for the complete index build.
//! The heavy work lives in `indexer`, `merger`, and `index`; this module selects
//! options, connects stages, records metadata, and cleans up successful builds.

use crate::{compression, index, indexer, merger, tokenizer};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

const HELP: &str = "Search: search-engine search --index <directory>

Indexing usage: search-engine <dataset.tsv> [options]

Options:
  --output <directory>      Output directory (default: ./output/index)
  --batch-bytes <number>    Serialized bytes per batch (default: 67108864)
  --block-size <number>     Postings per compressed block (default: 128)
  --merge-fan-in <number>   Maximum inputs per merge (default: 64; maximum: 128)
  --threads <number>       Run-building workers (default: 8)
  --merge-threads <number> Merge workers (default: 2; capped by open-file budget)
  --sort <cpu|gpu>          Posting sort backend (default: gpu; CPU-only builds default to cpu)
  --keep-intermediate      Keep intermediate files (default: remove on success)
  --help                   Show this help
";

struct Options {
    dataset: String,
    output: PathBuf,
    batch_bytes: usize,
    block_size: usize,
    merge_fan_in: usize,
    keep_intermediate: bool,
    threads: usize,
    merge_threads: usize,
    sort: String,
}

/// Build runs, merge/compress them, then prepare source offsets and corpus metadata.
pub fn run_index(args: impl Iterator<Item = String>) -> io::Result<()> {
    let Some(options) = parse_options(args)? else {
        return Ok(());
    };

    if !Path::new(&options.dataset).is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Dataset file not found",
        ));
    }

    let total_start = Instant::now();
    let sort_start = Instant::now();
    let sorter = indexer::sorting::create_sorter(&options.sort)?;
    let sort_init = sort_start.elapsed();
    fresh_directory(&options.output)?;
    let work = options.output.join("_work");
    fs::create_dir(&work)?;
    println!(
        "Workers: build {}, merge {}; batch threshold {} bytes per build worker",
        options.threads, options.merge_threads, options.batch_bytes
    );
    println!(
        "Sort backend: {}; initialization {:.2}s",
        sorter.description(),
        sort_init.as_secs_f64()
    );

    let stage = Instant::now();
    let run_count = indexer::build_runs_parallel(
        &options.dataset,
        utf8(&work)?,
        options.batch_bytes,
        &tokenizer::SimpleTokenizer,
        options.threads,
        sorter.as_ref(),
    )?;
    println!(
        "Build: {run_count} runs in {:.2}s",
        stage.elapsed().as_secs_f64()
    );
    // Enumerate only runs produced in this build, excluding stale or unrelated files.
    let runs: Vec<PathBuf> = (1..=run_count)
        .map(|number| work.join(format!("run_{number:04}.bin")))
        .collect();

    let stage = Instant::now();
    let runs = merger::prepare_runs(
        runs,
        &work,
        options.merge_fan_in,
        options.merge_threads,
        options.keep_intermediate,
    )?;
    println!(
        "Intermediate merges: {} runs ready in {:.2}s",
        runs.len(),
        stage.elapsed().as_secs_f64()
    );

    let stage = Instant::now();
    // Fuse the final merge with compression to avoid writing/reading a full merged run.
    let codec = compression::Simple9;
    let mut writer = indexer::PostingsWriter::new(&options.output, options.block_size, &codec)?;
    let posting_count = merger::merge_into(&runs, |term, doc_id, frequency| {
        writer.push(term, doc_id, frequency)
    })?;
    let term_count = writer.finish()?;
    fs::copy(
        work.join("page_table.bin"),
        options.output.join("page_table.bin"),
    )?;
    println!(
        "Final merge + compression: {posting_count} postings, {term_count} terms in {:.2}s",
        stage.elapsed().as_secs_f64()
    );
    let stage = Instant::now();
    // The page table still has legacy records; enrich them with byte offsets for snippets.
    let documents = index::prepare_passages(&options.output, &options.dataset)?;
    println!(
        "Passage offsets: {documents} documents in {:.2}s",
        stage.elapsed().as_secs_f64()
    );
    {
        let page_table = index::PageTable::load(options.output.join("page_table.bin"))?;

        let stats = index::CollectionStats::from_page_table(&page_table);

        stats.save(options.output.join("metadata.bin"))?;

        println!(
            "Metadata: {} documents, {} indexed tokens, average length {:.2}",
            stats.document_count,
            stats.total_doc_length,
            stats.average_doc_length(),
        );
    }

    // Only remove temporary files created inside this new output directory.
    if !options.keep_intermediate {
        fs::remove_dir_all(&work)?;
    }

    println!("Index: {}", options.output.display());
    println!("Total: {:.2}s", total_start.elapsed().as_secs_f64());
    Ok(())
}

fn parse_options(args: impl Iterator<Item = String>) -> io::Result<Option<Options>> {
    let mut args = args;
    let mut dataset = None;
    let mut output = PathBuf::from("./output/index");
    let mut batch_bytes = 64 * 1024 * 1024;
    let mut block_size = 128;
    let mut merge_fan_in = 64;
    let mut keep_intermediate = false;
    let mut threads = 8;
    let mut merge_threads = 2;
    let mut sort = String::from(if cfg!(feature = "gpu-sort") {
        "gpu"
    } else {
        "cpu"
    });

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                print!("{HELP}");
                return Ok(None);
            }
            "--output" => output = PathBuf::from(next_value(&mut args, "--output")?),
            "--batch-bytes" => batch_bytes = positive_number(&mut args, "--batch-bytes")?,
            "--block-size" => block_size = positive_number(&mut args, "--block-size")?,
            "--merge-fan-in" => merge_fan_in = positive_number(&mut args, "--merge-fan-in")?,
            "--threads" => threads = positive_number(&mut args, "--threads")?,
            "--merge-threads" => merge_threads = positive_number(&mut args, "--merge-threads")?,
            "--sort" => sort = next_value(&mut args, "--sort")?,
            "--keep-intermediate" => keep_intermediate = true,
            _ if arg.starts_with('-') => return Err(invalid(format!("Unknown option: {arg}"))),
            _ => {
                if dataset.replace(arg).is_some() {
                    return Err(invalid("Provide exactly one dataset filename"));
                }
            }
        }
    }

    if sort != "cpu" && sort != "gpu" {
        println!("--sort must be cpu or gpu, setting to default");
        sort = String::from("cpu");
    }
    let dataset = dataset.ok_or_else(|| invalid(HELP))?;
    if !(2..=merger::MAX_OPEN_INPUTS).contains(&merge_fan_in) {
        println!("--merge-fan-in must be between 2 and 128, setting to default");
        merge_fan_in = 64;
    }
    if u32::try_from(block_size).is_err() {
        println!("--block-size exceeds u32, setting to default");
        block_size = 128;
    }

    Ok(Some(Options {
        dataset,
        output,
        batch_bytes,
        block_size,
        merge_fan_in,
        keep_intermediate,
        threads,
        merge_threads,
        sort,
    }))
}

fn next_value(args: &mut impl Iterator<Item = String>, flag: &str) -> io::Result<String> {
    args.next()
        .ok_or_else(|| invalid(format!("Missing value for {flag}")))
}

fn positive_number(args: &mut impl Iterator<Item = String>, flag: &str) -> io::Result<usize> {
    let value = next_value(args, flag)?;
    let number = value
        .parse::<usize>()
        .map_err(|_| invalid(format!("Invalid value for {flag}: {value}")))?;
    if number == 0 {
        return Err(invalid(format!("{flag} must be greater than zero")));
    }
    Ok(number)
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

// Refuse an existing destination so old index files cannot mix with this build.
fn fresh_directory(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    fs::create_dir(path).map_err(|e| {
        if e.kind() == io::ErrorKind::AlreadyExists {
            invalid("Output directory exists; choose a new directory")
        } else {
            e
        }
    })
}
fn utf8(path: &Path) -> io::Result<&str> {
    path.to_str().ok_or_else(|| invalid("Path must be UTF-8"))
}
