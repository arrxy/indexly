use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct TestDir(PathBuf);
impl TestDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "parallel-index-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
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

fn run(args: &[&str]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_search-engine"))
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let started = Instant::now();
    loop {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        if started.elapsed() > Duration::from_secs(15) {
            child.kill().unwrap();
            let output = child.wait_with_output().unwrap();
            panic!(
                "Indexer did not terminate: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn build(input: &Path, output: &Path, threads: &str, merge_threads: &str) {
    let result = run(&[
        input.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
        "--batch-bytes",
        "512",
        "--block-size",
        "2",
        "--merge-fan-in",
        "3",
        "--threads",
        threads,
        "--merge-threads",
        merge_threads,
    ]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(!output.join("_work").exists());
}

#[test]
fn parallel_build_and_merge_match_serial_bytes_and_page_order() {
    let root = TestDir::new("equivalence");
    let input = root.0.join("input.tsv");
    let mut text = String::new();
    let mut expected_pages = Vec::new();
    for i in 0_u32..640 {
        let passage_id = i * 11 + 7;
        let repetitions = if i % 17 == 0 { 20_u32 } else { 1 };
        let body = if i % 31 == 0 {
            String::new()
        } else {
            "Rust rust! Café naïve 123. ".repeat(repetitions as usize)
        };
        text.push_str(&format!("{passage_id}\t{body}\n"));
        expected_pages.extend_from_slice(&passage_id.to_le_bytes());
        expected_pages.extend_from_slice(
            &(if body.is_empty() {
                0_u32
            } else {
                5 * repetitions
            })
            .to_le_bytes(),
        );
    }
    fs::write(&input, text).unwrap();
    let serial = root.0.join("serial");
    build(&input, &serial, "1", "1");
    assert_eq!(
        fs::read(serial.join("page_table.bin")).unwrap(),
        expected_pages
    );
    for (threads, merge_threads) in [("4", "1"), ("1", "3"), ("4", "3")] {
        let parallel = root.0.join(format!("parallel-{threads}-{merge_threads}"));
        build(&input, &parallel, threads, merge_threads);
        for name in ["postings.bin", "lexicon.bin", "page_table.bin"] {
            assert_eq!(
                fs::read(serial.join(name)).unwrap(),
                fs::read(parallel.join(name)).unwrap(),
                "{name}: build workers {threads}, merge workers {merge_threads}"
            );
        }
    }
}

#[test]
fn empty_collections_and_documents_without_tokens_match_serial() {
    let root = TestDir::new("empty");
    for (name, contents) in [("empty", ""), ("no_tokens", "8\t...\n99\t   \n")] {
        let input = root.0.join(format!("{name}.tsv"));
        fs::write(&input, contents).unwrap();
        let serial = root.0.join(format!("{name}-serial"));
        let parallel = root.0.join(format!("{name}-parallel"));
        build(&input, &serial, "1", "1");
        build(&input, &parallel, "4", "2");
        for file in ["postings.bin", "lexicon.bin", "page_table.bin"] {
            assert_eq!(
                fs::read(serial.join(file)).unwrap(),
                fs::read(parallel.join(file)).unwrap()
            );
        }
        assert_eq!(
            fs::metadata(parallel.join("postings.bin")).unwrap().len(),
            9
        );
        assert_eq!(fs::read(parallel.join("lexicon.bin")).unwrap(), b"LEX1");
    }
}

#[test]
fn input_errors_and_invalid_worker_options_terminate_with_errors() {
    let root = TestDir::new("errors");
    let input = root.0.join("bad.tsv");
    let mut text = (0..2000)
        .map(|i| format!("{i}\trust search\n"))
        .collect::<String>();
    text.push_str("missing tab\n");
    fs::write(&input, text).unwrap();
    let output = root.0.join("invalid-input");
    let result = run(&[
        input.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
        "--threads",
        "4",
        "--merge-threads",
        "2",
    ]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("Invalid line format"));
    assert!(!output.join("postings.bin").exists());
    for (flag, value) in [
        ("--threads", "0"),
        ("--merge-threads", "0"),
        ("--merge-fan-in", "129"),
    ] {
        let result = run(&[input.to_str().unwrap(), flag, value]);
        assert!(!result.status.success());
    }
    let help = run(&["--help"]);
    assert!(help.status.success());
    let help = String::from_utf8_lossy(&help.stdout);
    assert!(help.contains("--threads") && help.contains("--merge-threads"));
}
