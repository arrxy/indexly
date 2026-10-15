mod parallel;
pub(crate) use parallel::{merge_all, MAX_OPEN_INPUTS};

/*
Merges sorted binary runs into one globally sorted binary file.

Each run is ordered by (term, doc_id). A min-heap holds one posting
per active run. After writing a posting, only its source is advanced.

The output uses the same uncompressed record format as the input runs.
*/

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::indexer::read_next_posting;

pub fn merge_runs(run_paths: &[PathBuf], output_path: &Path) -> io::Result<u64> {
    let mut readers = run_paths
        .iter()
        .map(|path| File::open(path).map(BufReader::new))
        .collect::<io::Result<Vec<_>>>()?;
    let mut heap: BinaryHeap<Reverse<(String, u32, usize, u32)>> = BinaryHeap::new();
    for (run_id, reader) in readers.iter_mut().enumerate() {
        if let Some(posting) = read_next_posting(reader)? {
            heap.push(Reverse((posting.term, posting.doc_id, run_id, posting.term_frequency)));
        }
    }
    let mut output = BufWriter::new(File::create(output_path)?);
    let mut posting_count = 0_u64;
    while let Some(Reverse((term, doc_id, run_id, frequency))) = heap.pop() {
        let term_bytes = term.as_bytes();

        let term_length = u32::try_from(term_bytes.len()).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidData, "Term too long")
        })?;

        output.write_all(&term_length.to_le_bytes())?;
        output.write_all(term_bytes)?;
        output.write_all(&doc_id.to_le_bytes())?;
        output.write_all(&frequency.to_le_bytes())?;
        posting_count += 1;

        if let Some(posting) = read_next_posting(&mut readers[run_id])? {
            heap.push(Reverse((
                posting.term,
                posting.doc_id,
                run_id,
                posting.term_frequency,
            )));
        }
    }

    output.flush()?;
    Ok(posting_count)
}