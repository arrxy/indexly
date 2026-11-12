/* A single process handles successive queries and mode changes. */
use super::dat::{DEFAULT_TOP_K, validate_top_k};
use super::{QueryMode, SearchEngine};
use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::PathBuf;
use std::time::Instant;

const HELP: &str = "Usage: search --index <index-directory> [--mode and|or] [--dataset <TSV>]
              [--k1 <number>] [--b <number>] [--no-snippets] [--top-k <1-1000>]
              [--queries <TSV>]

--top-k sets the result limit per query (default: 10).
--queries reads headerless query_id<TAB>query_text rows and disables snippets.
Batch stdout is headerless query_id<TAB>passage_id<TAB>rank<TAB>score, with ranks starting at 1.

Commands inside the prompt:
  /mode and       Require all query terms (conjunctive)
  /mode or        Match any query term (disjunctive; default)
  /and, /or       Shortcuts for changing mode
  /help           Show commands
  /quit           Exit (EOF also exits)
Enter ordinary text to run a query. The process stays open after each query.
";

pub fn run_cli(args: impl Iterator<Item = String>) -> io::Result<()> {
    let mut args = args;
    let mut index = None;
    let mut dataset = None;
    let mut queries = None;
    let mut top_k = DEFAULT_TOP_K;
    let mut mode = QueryMode::Disjunctive;
    let mut snippets = true;
    let mut k1 = 1.2;
    let mut b = 0.75;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                print!("{HELP}");
                return Ok(());
            }
            "--index" => index = Some(PathBuf::from(value(&mut args, &arg)?)),
            "--dataset" => dataset = Some(PathBuf::from(value(&mut args, &arg)?)),
            "--queries" => queries = Some(PathBuf::from(value(&mut args, &arg)?)),
            "--top-k" => {
                top_k = value(&mut args, &arg)?
                    .parse()
                    .map_err(|_| invalid("--top-k requires a positive integer"))?;
            }
            "--mode" => mode = QueryMode::parse(&value(&mut args, &arg)?)?,
            "--k1" => k1 = number(&mut args, &arg)?,
            "--b" => b = number(&mut args, &arg)?,
            "--no-snippets" => snippets = false,
            _ => return Err(invalid(&format!("Unknown search option: {arg}\n{HELP}"))),
        }
    }
    let index = index.ok_or_else(|| invalid(HELP))?;
    validate_top_k(top_k)?;
    let started = Instant::now();
    let mut engine = SearchEngine::open(
        index,
        dataset.as_deref(),
        snippets && queries.is_none(),
        k1,
        b,
    )?;
    let stdout = io::stdout();
    if let Some(queries) = queries {
        let input = BufReader::new(File::open(queries)?);
        let mut output = BufWriter::new(stdout.lock());
        run_batch(&mut engine, input, &mut output, mode, top_k)?;
        return output.flush();
    }

    println!(
        "Loaded {} documents and {} terms in {:.2}s",
        engine.document_count(),
        engine.term_count(),
        started.elapsed().as_secs_f64()
    );
    let stdin = io::stdin();
    run_session_with_top_k(
        &mut engine,
        &mut stdin.lock(),
        &mut stdout.lock(),
        mode,
        top_k,
    )
}

pub fn run_session(
    engine: &mut SearchEngine,
    input: &mut impl BufRead,
    output: &mut impl Write,
    mode: QueryMode,
) -> io::Result<()> {
    run_session_with_top_k(engine, input, output, mode, DEFAULT_TOP_K)
}

fn run_session_with_top_k(
    engine: &mut SearchEngine,
    input: &mut impl BufRead,
    output: &mut impl Write,
    mut mode: QueryMode,
    top_k: usize,
) -> io::Result<()> {
    writeln!(
        output,
        "Search ready. Mode: {}. Use /mode and, /mode or, /help or /quit.",
        mode.label()
    )?;
    let mut line = String::new();
    loop {
        write!(output, "{}> ", mode.label())?;
        output.flush()?;
        line.clear();
        if input.read_line(&mut line)? == 0 {
            writeln!(output)?;
            break;
        }
        let query = line.trim();
        if query.is_empty() {
            continue;
        }
        if query.starts_with('/') {
            let parts: Vec<_> = query.split_whitespace().collect();
            match parts.as_slice() {
                ["/quit"] | ["/exit"] => break,
                ["/help"] => writeln!(output, "{HELP}")?,
                ["/and"] => {
                    mode = QueryMode::Conjunctive;
                    writeln!(output, "Mode: AND")?;
                }
                ["/or"] => {
                    mode = QueryMode::Disjunctive;
                    writeln!(output, "Mode: OR")?;
                }
                ["/mode", name] => match QueryMode::parse(name) {
                    Ok(next) => {
                        mode = next;
                        writeln!(output, "Mode: {}", mode.label())?;
                    }
                    Err(e) => writeln!(output, "{e}")?,
                },
                _ => writeln!(output, "Unknown command. Use /help for available commands.")?,
            }
            continue;
        }
        let started = Instant::now();
        let result = match engine.search_top_k(query, mode, top_k) {
            Ok(result) => result,
            Err(e) if e.kind() == io::ErrorKind::InvalidInput => {
                writeln!(output, "Query error: {e}")?;
                continue;
            }
            Err(e) => return Err(e),
        };
        writeln!(
            output,
            "Query: {query} | mode: {} | terms: {}",
            mode.label(),
            result.terms.join(", ")
        )?;
        if result.hits.is_empty() {
            writeln!(output, "No results.")?;
        }
        for (i, hit) in result.hits.iter().enumerate() {
            writeln!(
                output,
                "{}. passage_id={} doc_id={} score={:.6}",
                i + 1,
                hit.passage_id,
                hit.doc_id,
                hit.score
            )?;
            write!(output, "   terms: ")?;
            for (j, term) in hit.term_scores.iter().enumerate() {
                if j > 0 {
                    write!(output, ", ")?;
                }
                write!(
                    output,
                    "{}={:.6} (tf={})",
                    term.term, term.score, term.frequency
                )?;
            }
            writeln!(output)?;
            if !hit.snippet.is_empty() {
                writeln!(output, "   snippet: {}", hit.snippet)?;
            }
        }
        writeln!(
            output,
            "{} matching documents; showing {} results in {:.3}s\n",
            result.matched_documents,
            result.hits.len(),
            started.elapsed().as_secs_f64()
        )?;
    }
    Ok(())
}

fn run_batch(
    engine: &mut SearchEngine,
    input: impl BufRead,
    output: &mut impl Write,
    mode: QueryMode,
    top_k: usize,
) -> io::Result<()> {
    for (row, line) in input.lines().enumerate() {
        let line = line?;
        let line_number = row + 1;
        let (query_id, query) = line
            .split_once('\t')
            .filter(|(id, _)| !id.is_empty() && !id.chars().any(char::is_whitespace))
            .ok_or_else(|| {
                invalid(&format!(
                    "Query line {line_number}: expected query_id<TAB>query_text"
                ))
            })?;
        let result = engine.search_top_k(query, mode, top_k).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("Query line {line_number} ({query_id}): {error}"),
            )
        })?;
        for (rank, hit) in result.hits.iter().enumerate() {
            writeln!(
                output,
                "{query_id}\t{}\t{}\t{}",
                hit.passage_id,
                rank + 1,
                hit.score
            )?;
        }
    }
    Ok(())
}

fn value(args: &mut impl Iterator<Item = String>, flag: &str) -> io::Result<String> {
    args.next()
        .ok_or_else(|| invalid(&format!("Missing value for {flag}")))
}
fn number(args: &mut impl Iterator<Item = String>, flag: &str) -> io::Result<f64> {
    value(args, flag)?
        .parse()
        .map_err(|_| invalid(&format!("Invalid number for {flag}")))
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
