//! Merge sorted runs using one pending posting per input in a min-heap.
//! `merge_into` emits to a callback, allowing the last pass to compress directly.
//! `merge_runs` uses the same traversal to write uncompressed intermediate runs.

mod parallel;
pub use parallel::{MAX_OPEN_INPUTS, prepare_runs};

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::indexer::read_next_posting;

/// Emit sorted records and return their count without creating a final merged file.
/// Inputs must already be sorted and contain disjoint term/document pairs;
/// this traversal does not combine duplicate pairs across runs.
pub fn merge_into<F>(run_paths: &[PathBuf], mut emit: F) -> io::Result<u64>
where
    F: FnMut(&str, u32, u32) -> io::Result<()>,
{
    if run_paths.len() > MAX_OPEN_INPUTS {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Prepare runs before merging more than 128 inputs",
        ));
    }
    let mut readers = run_paths
        .iter()
        .map(|path| File::open(path).map(BufReader::new))
        .collect::<io::Result<Vec<_>>>()?;
    // Reverse makes BinaryHeap a min-heap. run_id identifies which reader to refill.
    let mut heap: BinaryHeap<Reverse<(String, u32, usize, u32)>> = BinaryHeap::new();
    for (run_id, reader) in readers.iter_mut().enumerate() {
        if let Some(posting) = read_next_posting(reader)? {
            heap.push(Reverse((
                posting.term,
                posting.doc_id,
                run_id,
                posting.term_frequency,
            )));
        }
    }
    let mut count = 0;
    while let Some(Reverse((term, doc_id, run_id, frequency))) = heap.pop() {
        emit(&term, doc_id, frequency)?;
        count += 1;
        // Only the winning run advances; every other run keeps its current minimum.
        if let Some(posting) = read_next_posting(&mut readers[run_id])? {
            heap.push(Reverse((
                posting.term,
                posting.doc_id,
                run_id,
                posting.term_frequency,
            )));
        }
    }
    Ok(count)
}

/// Write an intermediate run, flushing successfully before the caller can remove inputs.
pub fn merge_runs(run_paths: &[PathBuf], output_path: &Path) -> io::Result<u64> {
    let mut output = BufWriter::new(File::create(output_path)?);
    let count = merge_into(run_paths, |term, doc_id, frequency| {
        let bytes = term.as_bytes();
        let length = u32::try_from(bytes.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Term too long"))?;
        output.write_all(&length.to_le_bytes())?;
        output.write_all(bytes)?;
        output.write_all(&doc_id.to_le_bytes())?;
        output.write_all(&frequency.to_le_bytes())
    })?;
    output.flush()?;
    Ok(count)
}

#[cfg(test)]
#[path = "../../tests/unit/merger.rs"]
mod tests;
