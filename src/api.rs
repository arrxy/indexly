//! High-level indexing and search configuration.
use crate::indexer::sorting::PostingSorter;
use crate::retrieval::SearchEngine;
use crate::{compression::Simple9, index, indexer, merger, tokenizer::SimpleTokenizer};
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Posting-sort backend. CPU sorting is the portable default.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SortBackend {
    /// Sort posting batches on the CPU.
    #[default]
    Cpu,
    /// Use Metal through `metal-radix`; requires the `gpu-sort` feature and hardware.
    /// Enabling the feature does not select this backend automatically.
    Gpu,
}

impl SortBackend {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Gpu => "gpu",
        }
    }
}

/// Options shared by TSV and in-memory passage indexing.
///
/// `batch_bytes` is a serialized-posting threshold **per worker**, checked after
/// each passage, not a hard resident-memory limit. Strings, queues, sorting,
/// lexicons, and a single large passage need additional memory.
#[derive(Debug, Clone)]
pub struct IndexOptions {
    /// Run-building workers; must be positive. Default: 8.
    pub threads: usize,
    /// Independent intermediate-merge workers; must be positive. Default: 2.
    pub merge_threads: usize,
    /// Serialized posting bytes buffered per run-building worker. Default: 64 MiB.
    pub batch_bytes: usize,
    /// Postings per compressed block, in `1..=u32::MAX`. Default: 128.
    pub block_size: usize,
    /// Inputs per merge group, in `2..=128`. Default: 64.
    pub merge_fan_in: usize,
    /// Sort backend; default CPU, even in feature-enabled builds.
    pub sort: SortBackend,
    /// Preserve `_work` on success for inspecting runs. Default: false.
    pub keep_intermediate: bool,
}

impl Default for IndexOptions {
    fn default() -> Self {
        Self {
            threads: 8,
            merge_threads: 2,
            batch_bytes: 64 * 1024 * 1024,
            block_size: 128,
            merge_fan_in: 64,
            sort: SortBackend::Cpu,
            keep_intermediate: false,
        }
    }
}

impl IndexOptions {
    /// Validate numeric limits without creating files or initializing a GPU.
    pub fn validate(&self) -> io::Result<()> {
        if self.threads == 0 || self.merge_threads == 0 || self.batch_bytes == 0 {
            return Err(invalid(
                "--threads, --merge-threads and --batch-bytes must be positive",
            ));
        }
        if self.block_size == 0 || u32::try_from(self.block_size).is_err() {
            return Err(invalid("--block-size must be in 1..=u32::MAX"));
        }
        if !(2..=merger::MAX_OPEN_INPUTS).contains(&self.merge_fan_in) {
            return Err(invalid("--merge-fan-in must be between 2 and 128"));
        }
        Ok(())
    }
}

/// One searchable passage, supplied by an application after document extraction.
///
/// IDs are caller-provided `u32` values, distinct from row-order internal document
/// IDs. Text line endings are replaced with spaces when saving `passages.tsv`.
/// The engine does not perform PDF/DOCX extraction or passage chunking.
#[derive(Debug, Clone)]
pub struct Document {
    /// Caller-provided passage ID returned in search hits.
    pub id: u32,
    /// UTF-8 text to tokenize and retain for snippets and previews.
    pub text: String,
}

impl Document {
    /// Create a passage with an external ID and UTF-8 text.
    pub fn new(id: u32, text: impl Into<String>) -> Self {
        Self {
            id,
            text: text.into(),
        }
    }
}

/// Counts and wall-clock durations for a completed build.
#[derive(Debug)]
pub struct BuildReport {
    /// Newly created index directory.
    pub index_dir: PathBuf,
    /// Passage count, including passages with no tokens.
    pub documents: u64,
    /// Indexed token occurrences.
    pub tokens: u64,
    /// Unique indexed terms.
    pub terms: u64,
    /// Term/document posting count.
    pub postings: u64,
    /// Initial sorted-run count.
    pub run_count: usize,
    /// Sorter description, including adapter information in GPU mode.
    pub sort_backend: String,
    /// Sorter initialization time.
    pub sort_initialization: Duration,
    /// Run-building time.
    pub build_time: Duration,
    /// Intermediate-merge time, excluding final merge/compression.
    pub intermediate_merge_time: Duration,
    /// Final streamed merge and compression time.
    pub compression_time: Duration,
    /// Passage-offset preparation time.
    pub passage_offsets_time: Duration,
    /// Total time, including source preparation and metadata writes.
    pub elapsed: Duration,
}

/// Build an index from UTF-8 `u32_id<TAB>passage_text` rows.
///
/// The output directory must not already exist. The source TSV remains the text
/// store; keep it unchanged for snippets. No progress is printed by this function.
/// Numeric options, paths, and the sorter are checked before output creation.
/// On a returned build error, the newly created output directory is removed on a
/// best-effort basis. This function provides no collection locking or atomic
/// replacement; applications should stage a new directory and publish it themselves.
pub fn build_index(
    dataset: impl AsRef<Path>,
    output: impl AsRef<Path>,
    options: &IndexOptions,
) -> io::Result<BuildReport> {
    let source = fs::canonicalize(dataset)?;
    if !source.is_file() {
        return Err(invalid("Dataset must be a file"));
    }
    utf8(&source)?;
    build_new(output.as_ref(), options, |_| Ok(source))
}

/// Index a stream of passages, retaining their text as `output/passages.tsv`.
///
/// The iterator is consumed incrementally. Newlines and carriage returns in each
/// passage become spaces; tabs are preserved. Empty passages are supported. Use
/// stable unique external IDs when the application needs unambiguous source lookup.
/// The same output/error rules as [`build_index`] apply.
pub fn index_documents(
    documents: impl IntoIterator<Item = Document>,
    output: impl AsRef<Path>,
    options: &IndexOptions,
) -> io::Result<BuildReport> {
    build_new(output.as_ref(), options, |output| {
        let source = output.join("passages.tsv");
        let mut writer = BufWriter::new(File::create(&source)?);
        for document in documents {
            writeln!(
                writer,
                "{}\t{}",
                document.id,
                document.text.replace(['\r', '\n'], " ")
            )?;
        }
        writer.flush()?;
        fs::canonicalize(source)
    })
}

fn build_new(
    output: &Path,
    options: &IndexOptions,
    source: impl FnOnce(&Path) -> io::Result<PathBuf>,
) -> io::Result<BuildReport> {
    let start = Instant::now();
    options.validate()?;
    utf8(output)?;
    let sort_start = Instant::now();
    let sorter = indexer::sorting::create_sorter(options.sort.label())?;
    let sort_initialization = sort_start.elapsed();
    if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    fs::create_dir(output).map_err(|error| {
        if error.kind() == io::ErrorKind::AlreadyExists {
            invalid("Output directory already exists; choose a new output directory")
        } else {
            error
        }
    })?;
    let result = (|| {
        let source = source(output)?;
        build_into(
            &source,
            output,
            options,
            sorter.as_ref(),
            sort_initialization,
            start,
        )
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(output);
    }
    result
}

fn build_into(
    source: &Path,
    output: &Path,
    options: &IndexOptions,
    sorter: &dyn PostingSorter,
    sort_initialization: Duration,
    start: Instant,
) -> io::Result<BuildReport> {
    let work = output.join("_work");
    fs::create_dir(&work)?;
    let stage = Instant::now();
    let run_count = indexer::build_runs_parallel(
        utf8(source)?,
        utf8(&work)?,
        options.batch_bytes,
        &SimpleTokenizer,
        options.threads,
        sorter,
    )?;
    let build_time = stage.elapsed();
    let runs = (1..=run_count)
        .map(|n| work.join(format!("run_{n:04}.bin")))
        .collect();
    let stage = Instant::now();
    let runs = merger::prepare_runs(
        runs,
        &work,
        options.merge_fan_in,
        options.merge_threads,
        options.keep_intermediate,
    )?;
    let intermediate_merge_time = stage.elapsed();
    let stage = Instant::now();
    let mut writer = indexer::PostingsWriter::new(output, options.block_size, &Simple9)?;
    let postings = merger::merge_into(&runs, |term, doc, freq| writer.push(term, doc, freq))?;
    let terms = writer.finish()?;
    fs::copy(work.join("page_table.bin"), output.join("page_table.bin"))?;
    let compression_time = stage.elapsed();
    let stage = Instant::now();
    index::prepare_passages(output, source)?;
    let passage_offsets_time = stage.elapsed();
    let table = index::PageTable::load(output.join("page_table.bin"))?;
    let stats = index::CollectionStats::from_page_table(&table);
    stats.save(output.join("metadata.bin"))?;
    if !options.keep_intermediate {
        fs::remove_dir_all(work)?;
    }
    Ok(BuildReport {
        index_dir: output.to_owned(),
        documents: stats.document_count,
        tokens: stats.total_doc_length,
        terms,
        postings,
        run_count,
        sort_backend: sorter.description().to_owned(),
        sort_initialization,
        build_time,
        intermediate_merge_time,
        compression_time,
        passage_offsets_time,
        elapsed: start.elapsed(),
    })
}

/// Search configuration; defaults to snippets enabled and BM25 `k1=1.2, b=0.75`.
#[derive(Debug, Clone)]
pub struct SearchOptions {
    /// Generate snippets and enable full-passage retrieval. Default: true.
    pub snippets: bool,
    /// Override the saved TSV location if the identical source file moved.
    pub dataset: Option<PathBuf>,
    /// BM25 term-frequency saturation, finite and nonnegative. Default: 1.2.
    pub k1: f64,
    /// BM25 length normalization in `0..=1`. Default: 0.75.
    pub b: f64,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            snippets: true,
            dataset: None,
            k1: 1.2,
            b: 0.75,
        }
    }
}

impl SearchOptions {
    /// Open an existing index with these options. Does not acquire collection locks.
    /// Legacy page tables without text offsets require `snippets: false`.
    pub fn open(&self, index_dir: impl AsRef<Path>) -> io::Result<SearchEngine> {
        SearchEngine::open(
            index_dir,
            self.dataset.as_deref(),
            self.snippets,
            self.k1,
            self.b,
        )
    }
}

fn utf8(path: &Path) -> io::Result<&str> {
    path.to_str()
        .ok_or_else(|| invalid("Index and dataset paths must be UTF-8"))
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
