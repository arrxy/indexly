/*
This module sorts intermediate postings by term, then by doc_id,
and writes each batch to a binary run file.

Each record is stored as:
[term byte length: u32][UTF-8 term bytes][doc_id: u32][term_frequency: u32]

Integer fields use little-endian byte order. Writes are buffered
for efficient disk access.

These sorted run files are uncompressed. They will later be merged
to create the final compressed inverted index.
*/
use crate::indexer::builder::IntermediatePosting;
use std::fs::File;
use std::io::{self, BufWriter, Write};
pub fn write_run(file_path: &str, postings: &mut [IntermediatePosting]) -> io::Result<()> {
    // Sort postings by term, then by doc_id
    postings.sort_by(|a, b| {
        let term_cmp = a.term.cmp(&b.term);
        if term_cmp == std::cmp::Ordering::Equal {
            a.doc_id.cmp(&b.doc_id)
        } else {
            term_cmp
        }
    });
    let file = File::create(file_path)?;
    let mut writer = BufWriter::new(file);
    for posting in postings {
        let term_bytes = posting.term.as_bytes();
        let term_length = u32::try_from(term_bytes.len()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "Term exceeds the maximum byte length",
            )
        })?;
        writer.write_all(&term_length.to_le_bytes())?;
        writer.write_all(term_bytes)?;
        writer.write_all(&posting.doc_id.to_le_bytes())?;
        writer.write_all(&posting.term_frequency.to_le_bytes())?;
    }
    Ok(())
}