//! Read uncompressed run records: term_length:u32, UTF-8 term, doc_id:u32, tf:u32.
//! Integer fields are little-endian. The merger keeps only one record per run in
//! its heap, so it never needs to load a complete run.

use super::builder::IntermediatePosting;
use crate::binary::read_u32;
use std::io;
use std::io::BufRead;

/// Return None only at a record boundary; incomplete records propagate I/O errors.
pub fn read_next_posting(reader: &mut impl BufRead) -> io::Result<Option<IntermediatePosting>> {
    if reader.fill_buf()?.is_empty() {
        return Ok(None);
    }
    let term_length = read_u32(reader)?;
    let mut term_bytes = vec![0; term_length as usize];
    reader.read_exact(&mut term_bytes)?;
    let term = String::from_utf8(term_bytes).map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidData, "Invalid UTF-8 sequence in term")
    })?;
    let doc_id = read_u32(reader)?;
    let term_frequency = read_u32(reader)?;
    Ok(Some(IntermediatePosting {
        term,
        doc_id,
        term_frequency,
    }))
}
