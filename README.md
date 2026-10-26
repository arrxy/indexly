# Indexly

A disk-backed BM25 passage search engine in Rust, used as the search core of
[Indexify](https://github.com/arrxy/indexify). Indexly handles tokenization,
parallel indexing, external merging, compressed postings, and exact AND/OR
retrieval. Applications handle document extraction, passage chunking, source
metadata, collection locking, and user interfaces.

Requires Rust 1.88 or later. CPU sorting is the default. Metal GPU sorting is an optional experiment selected
explicitly; ordinary library builds do not compile GPU dependencies.

## Use as a library

After the first crates.io release:

```toml
[dependencies]
indexly = "0.1.0"
```

Before publication, use a local checkout:

```toml
[dependencies]
indexly = { path = "../search-engine" }
```

```rust,no_run
use indexly::{Document, IndexOptions, QueryMode, SearchEngine, index_documents};

fn main() -> std::io::Result<()> {
    let documents = [
        Document::new(10, "Rust supports fast document search."),
        Document::new(20, "Compressed posting lists stay on disk."),
    ];
    // The output must be a new directory; existing directories are never replaced.
    let report = index_documents(documents, "my-new-index", &IndexOptions::default())?;
    let mut engine = SearchEngine::open_default(&report.index_dir)?;
    let results = engine.search("rust search", QueryMode::Conjunctive)?;
    for hit in results.hits {
        println!("{}: {:.4} {}", hit.passage_id, hit.score, hit.snippet);
        println!("{}", engine.passage_text(hit.doc_id)?);
    }
    Ok(())
}
```

`Document` represents one passage. Supply stable unique `u32` IDs for source
lookup; IDs need not be contiguous. The iterator is consumed incrementally and
text is retained in `my-new-index/passages.tsv`. Line endings become spaces;
tabs and Unicode text are preserved. Indexly does not open PDFs or DOCX files.

For a UTF-8 TSV containing one `u32_id<TAB>text` row per passage:

```rust,no_run
use indexly::{IndexOptions, QueryMode, SearchOptions, build_index};

fn main() -> std::io::Result<()> {
    let options = IndexOptions {
        threads: 4,
        merge_threads: 2,
        ..IndexOptions::default()
    };
    let report = build_index("passages.tsv", "new-index", &options)?;
    let mut engine = SearchOptions {
        snippets: false,
        ..SearchOptions::default()
    }.open(&report.index_dir)?;
    let results = engine.search("compressed search", QueryMode::Disjunctive)?;
    println!("{} matches", results.matched_documents);
    Ok(())
}
```

`build_index` retains the original TSV as its text store rather than duplicating
it. Keep that file unchanged for snippets and full-passage retrieval. If the
identical file moves, set `SearchOptions::dataset` to its new location. Source
size is checked; timestamps are also checked when using the saved location.
The relocation override is not a content checksum.

Both entry points orchestrate run construction, merging, compression, passage
offsets, metadata, and temporary-run cleanup. They return counts and timings in
`BuildReport` and print nothing. On a returned build error, the newly created
output is removed on a best-effort basis. Existing directories are rejected.
Building is not an atomic replacement operation and acquires no collection lock;
applications should stage generations and coordinate publication/readers.

## Retrieval and limits

- Exact BM25 top 10; AND requires every distinct term, OR accepts any term.
- Defaults: `k1=1.2`, `b=0.75`; override with `SearchOptions`.
- At most 64 distinct normalized query terms; repeated terms count once.
- Equal scores break by internal document ID. Hits include external passage ID,
  internal row-order ID, total score, per-term contributions, and optional snippet.
- Unicode lowercase/alphanumeric tokenization; no stemming, phrase queries,
  semantic retrieval, or positional index.
- Compressed posting lists stay on disk. The lexicon and document table are
  memory-resident; each query-term cursor decodes one block at a time.
- `batch_bytes` is a per-worker serialized-posting threshold, not a RAM cap.
  Worker queues, strings, sorting buffers, and long passages use additional memory.
- Default build settings: 8 workers, 2 merge workers, 64 MiB batch threshold,
  128 postings/block, and merge fan-in 64. Merge fan-in supports 2--128 inputs;
  merge concurrency is capped by an input-file budget.

## Optional Metal sorting

```toml
[dependencies]
indexly = { version = "0.1.0", features = ["gpu-sort"] }
```

Select the backend explicitly:

```rust
use indexly::{IndexOptions, SortBackend};
let options = IndexOptions { sort: SortBackend::Gpu, ..IndexOptions::default() };
```

Metal sorting uses the published `metal-radix` dependency and requires a working
macOS Metal GPU. CPU remains the default even when the feature is enabled.
Requesting GPU mode without the feature or usable hardware returns an error;
there is no silent fallback. CPU and GPU use the same persisted index format.

## CLI and runnable examples

The existing executable names are preserved:

```sh
cargo build --release
./target/release/search-engine passages.tsv --output output/new-index
./target/release/search --index output/new-index --mode and

# Explicit GPU build and execution on macOS:
cargo build --release --features gpu-sort
./target/release/search-engine passages.tsv --output output/new-gpu-index --sort gpu

# Library examples; output directories must not exist:
cargo run --example documents -- /tmp/indexly-documents-demo
cargo run --example tsv -- passages.tsv /tmp/indexly-tsv-demo
```

Separate `build_runs` and `merge_index` executables remain available. See
[engine details](docs/engine-details.md) for staging, cursor traversal, merging,
and CPU/GPU comparison commands.

## Indexify integration

To retain Indexify's existing `use search_engine::...` imports, alias the package:

```toml
[dependencies]
search-engine = { package = "indexly", version = "0.1.0" }
```

For local development use `package = "indexly", path = "../search-engine"`.
After publication, replace the vendored engine dependency with the registry
version and remove the old `engine` workspace member/source copy. An external
sibling path is for development; it must not be shipped in a Homebrew source
archive. The low-level modules remain available for existing consumers, while
new applications should use the root-level API.

## Compatibility and release checks

This release preserves the existing compressed index format, including Indexify
indexes. Crate SemVer and index-format versions are separate: a version bump
alone neither guarantees compatibility nor requires rebuilding. See
[index-format compatibility](docs/index-format.md) for supported headers,
legacy page-table upgrades, and text-store requirements.

```sh
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --features gpu-sort
cargo doc --locked --no-deps
cargo package --locked --list
cargo publish --locked --dry-run
```

Hardware tests are ignored in the ordinary feature-enabled suite; run them on
macOS with `cargo test --locked --features gpu-sort -- --ignored`.
See the [release checklist](docs/releasing.md) for archive and consumer checks.

## License

[MIT](LICENSE). Copyright (c) 2026 Aritro Roy.
