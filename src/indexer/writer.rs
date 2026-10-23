//! Sort and serialize an intermediate posting batch.
//! CPU sorting rearranges records; GPU sorting supplies a permutation of their
//! original indices. Both paths write the same little-endian run format.

use super::sorting::{PostingOrder, PostingSorter};
use crate::indexer::builder::IntermediatePosting;
use std::fs::File;
use std::io::{self, BufWriter, Write};
/// Write a sorter-provided order, trusting indices to be a complete valid permutation.
pub fn write_run(
    file_path: &str,
    postings: &mut [IntermediatePosting],
    sorter: &dyn PostingSorter,
) -> io::Result<()> {
    let order = sorter.sort(postings)?;
    let file = File::create(file_path)?;
    let mut writer = BufWriter::new(file);
    match order {
        PostingOrder::InPlace => {
            for posting in postings {
                write_posting(&mut writer, posting)?;
            }
        }
        #[cfg(any(feature = "gpu-sort", test))]
        // Follow numeric indices directly, avoiding a second copy/reorder of owned strings.
        PostingOrder::Indices(indices) => {
            for index in indices {
                write_posting(&mut writer, &postings[index as usize])?;
            }
        }
    }
    writer.flush()
}

fn write_posting(writer: &mut impl Write, posting: &IntermediatePosting) -> io::Result<()> {
    let term_bytes = posting.term.as_bytes();
    // Trusted dataset terms fit the on-disk u32 length field.
    writer.write_all(&(term_bytes.len() as u32).to_le_bytes())?;
    writer.write_all(term_bytes)?;
    writer.write_all(&posting.doc_id.to_le_bytes())?;
    writer.write_all(&posting.term_frequency.to_le_bytes())
}

#[cfg(test)]
#[path = "../../tests/unit/indexer/writer.rs"]
mod tests;
