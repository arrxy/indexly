use indexly::{
    Document, IndexOptions, QueryMode, SearchEngine, SearchOptions, SortBackend, build_index,
    index_documents,
};
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

struct TestDir(PathBuf);
impl TestDir {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "indexly-api-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn options() -> IndexOptions {
    IndexOptions {
        threads: 3,
        merge_threads: 2,
        batch_bytes: 1,
        block_size: 2,
        merge_fan_in: 2,
        ..IndexOptions::default()
    }
}

#[test]
fn streaming_documents_support_text_ids_modes_and_empty_passages() {
    let root = TestDir::new();
    let output = root.0.join("index");
    let texts = [
        "Rust rust café\nsearch",
        "Rust\t東京",
        "search engines",
        "...",
    ];
    let report = index_documents(
        texts
            .into_iter()
            .enumerate()
            .map(|(i, text)| Document::new(i as u32 * 17 + 3, text)),
        &output,
        &options(),
    )
    .unwrap();
    assert_eq!(report.documents, 4);
    assert_eq!(report.tokens, 8);
    assert_eq!(report.terms, 5);
    assert_eq!(report.postings, 7);
    assert!(!output.join("_work").exists());
    assert!(output.join("passages.tsv").exists());
    let mut engine = SearchEngine::open_default(&output).unwrap();
    let and = engine
        .search("RUST search", QueryMode::Conjunctive)
        .unwrap();
    assert_eq!(and.matched_documents, 1);
    assert_eq!(and.hits[0].passage_id, 3);
    assert_eq!(
        engine.passage_text(and.hits[0].doc_id).unwrap(),
        "Rust rust café search"
    );
    assert_eq!(engine.passage_text(1).unwrap(), "Rust\t東京");
    assert_eq!(
        engine
            .search("rust search", QueryMode::Disjunctive)
            .unwrap()
            .matched_documents,
        3
    );
    assert_eq!(
        engine.passage_text(999).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    let mut ranking_only = SearchOptions {
        snippets: false,
        ..SearchOptions::default()
    }
    .open(&output)
    .unwrap();
    assert_eq!(
        ranking_only
            .search("rust", QueryMode::Disjunctive)
            .unwrap()
            .hits
            .len(),
        2
    );
    assert!(ranking_only.passage_text(0).is_err());
}

#[test]
fn library_and_cli_build_identical_files_from_the_same_source() {
    let root = TestDir::new();
    let source = root.0.join("source.tsv");
    fs::write(&source, "7\tRust rust café\n21\tsearch rust\n9\t...\n").unwrap();
    let library = root.0.join("library");
    let cli = root.0.join("cli");
    build_index(&source, &library, &options()).unwrap();
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_search-engine"))
        .arg(&source)
        .arg("--output")
        .arg(&cli)
        .args([
            "--threads",
            "3",
            "--merge-threads",
            "2",
            "--batch-bytes",
            "1",
            "--block-size",
            "2",
            "--merge-fan-in",
            "2",
        ])
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    for file in [
        "postings.bin",
        "lexicon.bin",
        "blocks.bin",
        "page_table.bin",
        "metadata.bin",
    ] {
        assert_eq!(
            fs::read(library.join(file)).unwrap(),
            fs::read(cli.join(file)).unwrap(),
            "{file}"
        );
    }
    assert_eq!(IndexOptions::default().sort, SortBackend::Cpu);
}

#[test]
fn invalid_options_do_not_create_output_and_existing_data_is_preserved() {
    let root = TestDir::new();
    for (i, options) in [
        IndexOptions {
            threads: 0,
            ..options()
        },
        IndexOptions {
            merge_threads: 0,
            ..options()
        },
        IndexOptions {
            batch_bytes: 0,
            ..options()
        },
        IndexOptions {
            block_size: 0,
            ..options()
        },
        IndexOptions {
            merge_fan_in: 1,
            ..options()
        },
        IndexOptions {
            merge_fan_in: 129,
            ..options()
        },
    ]
    .into_iter()
    .enumerate()
    {
        let output = root.0.join(i.to_string());
        assert_eq!(
            index_documents([], &output, &options).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert!(!output.exists());
    }
    let existing = root.0.join("existing");
    fs::create_dir(&existing).unwrap();
    fs::write(existing.join("keep"), b"untouched").unwrap();
    assert!(index_documents([], &existing, &options()).is_err());
    assert_eq!(fs::read(existing.join("keep")).unwrap(), b"untouched");
}

#[test]
fn source_errors_clean_new_output_and_empty_streams_work() {
    let root = TestDir::new();
    let input = root.0.join("bad.tsv");
    fs::write(&input, "7\trust\nno tab\n").unwrap();
    let output = root.0.join("bad");
    assert!(build_index(&input, &output, &options()).is_err());
    assert!(!output.exists());
    assert!(input.exists());
    let output = root.0.join("empty");
    assert_eq!(
        index_documents([], &output, &options()).unwrap().documents,
        0
    );
    let mut engine = SearchEngine::open_default(&output).unwrap();
    assert!(
        engine
            .search("rust", QueryMode::Disjunctive)
            .unwrap()
            .hits
            .is_empty()
    );
}

#[test]
fn moved_text_store_and_invalid_bm25_settings_are_handled() {
    let root = TestDir::new();
    let source = root.0.join("input.tsv");
    fs::write(&source, "10\trust search\n").unwrap();
    let output = root.0.join("index");
    build_index(&source, &output, &options()).unwrap();
    let moved = root.0.join("moved.tsv");
    fs::rename(source, &moved).unwrap();
    assert!(SearchEngine::open_default(&output).is_err());
    let mut engine = SearchOptions {
        dataset: Some(moved),
        ..SearchOptions::default()
    }
    .open(&output)
    .unwrap();
    assert_eq!(engine.passage_text(0).unwrap(), "rust search");
    for opts in [
        SearchOptions {
            snippets: false,
            k1: f64::NAN,
            ..SearchOptions::default()
        },
        SearchOptions {
            snippets: false,
            b: 1.1,
            ..SearchOptions::default()
        },
    ] {
        assert!(opts.open(&output).is_err());
    }
}

#[cfg(not(feature = "gpu-sort"))]
#[test]
fn requesting_gpu_without_feature_does_not_create_output() {
    let root = TestDir::new();
    let output = root.0.join("gpu");
    let error = index_documents(
        [],
        &output,
        &IndexOptions {
            sort: SortBackend::Gpu,
            ..options()
        },
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Unsupported);
    assert!(error.to_string().contains("--features gpu-sort"));
    assert!(!output.exists());
}

#[test]
fn pre_refactor_index_is_readable_and_writer_format_is_unchanged() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy-0.1.0");
    let source = fixture.join("passages.tsv");
    // The original fixture records its build-time absolute path. An identical
    // relocated source is explicitly supplied, as a packaged consumer would do.
    let mut old = SearchOptions {
        dataset: Some(source.clone()),
        ..SearchOptions::default()
    }
    .open(&fixture)
    .unwrap();
    assert_eq!(old.document_count(), 4);
    assert_eq!(old.term_count(), 8);
    let result = old.search("rust search", QueryMode::Conjunctive).unwrap();
    assert_eq!(result.matched_documents, 2);
    assert!(
        result
            .hits
            .iter()
            .all(|hit| [7, 90].contains(&hit.passage_id))
    );
    assert_eq!(old.passage_text(0).unwrap(), "Rust rust café search.");
    let root = TestDir::new();
    let output = root.0.join("new");
    let report = build_index(
        &source,
        &output,
        &IndexOptions {
            threads: 1,
            merge_threads: 1,
            batch_bytes: 96,
            block_size: 2,
            merge_fan_in: 2,
            ..IndexOptions::default()
        },
    )
    .unwrap();
    assert_eq!(report.documents, 4);
    for file in ["postings.bin", "lexicon.bin", "blocks.bin", "metadata.bin"] {
        assert_eq!(
            fs::read(fixture.join(file)).unwrap(),
            fs::read(output.join(file)).unwrap(),
            "{file}"
        );
    }
    let old_table = indexly::index::PageTable::load(fixture.join("page_table.bin")).unwrap();
    let new_table = indexly::index::PageTable::load(output.join("page_table.bin")).unwrap();
    for (old, new) in old_table.entries.iter().zip(new_table.entries.iter()) {
        assert_eq!(
            (
                old.passage_id,
                old.doc_length,
                old.passage_offset,
                old.passage_length
            ),
            (
                new.passage_id,
                new.doc_length,
                new.passage_offset,
                new.passage_length
            )
        );
    }
}
