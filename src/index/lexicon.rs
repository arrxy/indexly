//! In-memory term lookup loaded from the LEX2 dictionary.
//! Each entry stores: term length (u32), UTF-8 term bytes, postings offset (u64),
//! postings length (u64), document frequency (u32), blocks offset (u64), and
//! block count (u32). Integers are little-endian; offsets are absolute file bytes.

use crate::binary::{invalid, read_u32, read_u64};
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};
use std::path::Path;

/// Term-to-location lookup; posting payloads remain on disk.
#[derive(Debug)]
pub struct Lexicon {
    pub terms: HashMap<String, LexiconEntry>,
}

/// Locations and counts for one term, written by `PostingsWriter`.
#[derive(Debug)]
pub struct LexiconEntry {
    /// Absolute byte offset of the first block header in postings.bin.
    pub offset: u64,
    /// Total encoded posting-list bytes, including block headers.
    pub length: u64,
    /// Number of documents containing the term, not its total occurrences.
    pub doc_freq: u32,
    /// Absolute byte offset of this term's first directory record in blocks.bin.
    pub blocks_offset: u64,
    /// Number of 12-byte records in this term's block directory.
    pub block_count: u32,
}

impl Lexicon {
    /// Load a trusted LEX2 file; uniqueness, counts, and ranges are not revalidated.
    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        let file = File::open(path)?;
        let mut reader = BufReader::new(file);

        let mut magic = [0; 4];
        reader.read_exact(&mut magic)?;

        if &magic != b"LEX2" {
            return Err(invalid(
                "Expected LEX2 lexicon; rebuild older indexes to add block directories",
            ));
        }

        let mut terms = HashMap::new();

        // EOF is valid only between entries; read_exact reports a truncated entry.
        while !reader.fill_buf()?.is_empty() {
            let term_length = read_u32(&mut reader)?;
            let mut term_bytes = vec![0; term_length as usize];
            reader.read_exact(&mut term_bytes)?;

            let term = String::from_utf8(term_bytes)
                .map_err(|_| invalid("Invalid UTF-8 in lexicon term"))?;

            let entry = LexiconEntry {
                offset: read_u64(&mut reader)?,
                length: read_u64(&mut reader)?,
                doc_freq: read_u32(&mut reader)?,
                blocks_offset: read_u64(&mut reader)?,
                block_count: read_u32(&mut reader)?,
            };

            terms.insert(term, entry);
        }

        Ok(Self { terms })
    }
}
